/**
 * FaucetScreen — Claim testnet EVAP tokens
 *
 * Shows current balance, claim button with cooldown,
 * and recent claim history.
 */

import React, { useState, useEffect, useCallback } from 'react';
import {
  View,
  Text,
  TouchableOpacity,
  StyleSheet,
  Alert,
  ActivityIndicator,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import AsyncStorage from '@react-native-async-storage/async-storage';
import { api } from '../utils/api';
import { keystore } from '../utils/keystore';

const COOLDOWN_MS = 60 * 60 * 1000; // 1 hour between claims
const STORAGE_KEY = 'evap_last_faucet_claim';

const FaucetScreen: React.FC = () => {
  const [address, setAddress] = useState('');
  const [balance, setBalance] = useState<number | null>(null);
  const [claiming, setClaiming] = useState(false);
  const [lastClaim, setLastClaim] = useState<number>(0);
  const [now, setNow] = useState(Date.now());

  const loadData = useCallback(async () => {
    const addr = await keystore.getAddress();
    if (!addr) return;
    setAddress(addr);

    try {
      const bal = await api.getBalance(addr);
      setBalance(bal.balance);
    } catch {
      // Keep null
    }

    const stored = await AsyncStorage.getItem(STORAGE_KEY);
    if (stored) setLastClaim(parseInt(stored, 10));
  }, []);

  useEffect(() => {
    loadData();
  }, [loadData]);

  // Update timer every second
  useEffect(() => {
    const interval = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(interval);
  }, []);

  const cooldownRemaining = Math.max(0, COOLDOWN_MS - (now - lastClaim));
  const canClaim = cooldownRemaining === 0;

  const formatCooldown = (ms: number): string => {
    const minutes = Math.floor(ms / 60000);
    const seconds = Math.floor((ms % 60000) / 1000);
    return `${minutes}:${seconds.toString().padStart(2, '0')}`;
  };

  const handleClaim = async () => {
    if (!address || !canClaim) return;

    setClaiming(true);
    try {
      const result = await api.claimFaucet(address);
      if (result.success) {
        const claimTime = Date.now();
        setLastClaim(claimTime);
        await AsyncStorage.setItem(STORAGE_KEY, claimTime.toString());
        setBalance(result.balance);
        Alert.alert(
          'Tokens Received',
          `Your balance is now ${result.balance} EVAP.`
        );
      } else {
        Alert.alert('Claim Failed', result.message || 'Please try again later.');
      }
    } catch {
      Alert.alert('Error', 'Could not reach the faucet. Check your connection.');
    } finally {
      setClaiming(false);
    }
  };

  return (
    <SafeAreaView style={styles.container} edges={['bottom']}>
      <View style={styles.content}>
        {/* Faucet Icon */}
        <View style={styles.iconCircle}>
          <Text style={styles.iconText}>F</Text>
        </View>

        <Text style={styles.heading}>Testnet Faucet</Text>
        <Text style={styles.description}>
          Claim free EVAP tokens on the testnet to explore the network, send
          transactions, and interact with dApps.
        </Text>

        {/* Balance Card */}
        <View style={styles.balanceCard}>
          <Text style={styles.balanceLabel}>Current Balance</Text>
          <Text style={styles.balanceAmount}>
            {balance !== null ? balance.toLocaleString(undefined, { maximumFractionDigits: 4 }) : '---'} EVAP
          </Text>
        </View>

        {/* Address */}
        <View style={styles.addressBox}>
          <Text style={styles.addressLabel}>Receiving Address</Text>
          <Text style={styles.addressText} numberOfLines={1}>
            {address || 'Loading...'}
          </Text>
        </View>

        {/* Claim Button */}
        <TouchableOpacity
          style={[styles.claimButton, !canClaim && styles.claimButtonDisabled]}
          onPress={handleClaim}
          disabled={!canClaim || claiming}
          activeOpacity={0.7}
        >
          {claiming ? (
            <ActivityIndicator color="#ffffff" />
          ) : canClaim ? (
            <Text style={styles.claimButtonText}>Claim 100 EVAP</Text>
          ) : (
            <Text style={styles.claimButtonText}>
              Next claim in {formatCooldown(cooldownRemaining)}
            </Text>
          )}
        </TouchableOpacity>

        {/* Info */}
        <View style={styles.infoBox}>
          <View style={styles.infoRow}>
            <Text style={styles.infoLabel}>Amount per claim</Text>
            <Text style={styles.infoValue}>100 EVAP</Text>
          </View>
          <View style={styles.infoRow}>
            <Text style={styles.infoLabel}>Cooldown</Text>
            <Text style={styles.infoValue}>1 hour</Text>
          </View>
          <View style={styles.infoRow}>
            <Text style={styles.infoLabel}>Network</Text>
            <Text style={styles.infoValue}>Testnet</Text>
          </View>
        </View>

        <Text style={styles.disclaimer}>
          Testnet tokens have no monetary value. They are for testing only.
        </Text>
      </View>
    </SafeAreaView>
  );
};

const styles = StyleSheet.create({
  container: {
    flex: 1,
    backgroundColor: '#f9fafb',
  },
  content: {
    flex: 1,
    padding: 24,
    alignItems: 'center',
  },
  iconCircle: {
    width: 64,
    height: 64,
    borderRadius: 32,
    backgroundColor: '#06b6d4',
    alignItems: 'center',
    justifyContent: 'center',
    marginBottom: 16,
  },
  iconText: {
    color: '#ffffff',
    fontSize: 28,
    fontWeight: '800',
  },
  heading: {
    fontSize: 24,
    fontWeight: '700',
    color: '#111827',
    marginBottom: 8,
  },
  description: {
    fontSize: 14,
    color: '#6b7280',
    textAlign: 'center',
    lineHeight: 20,
    marginBottom: 24,
  },
  balanceCard: {
    backgroundColor: '#ffffff',
    borderRadius: 14,
    padding: 20,
    width: '100%',
    alignItems: 'center',
    marginBottom: 16,
    shadowColor: '#000',
    shadowOffset: { width: 0, height: 1 },
    shadowOpacity: 0.04,
    shadowRadius: 4,
    elevation: 1,
  },
  balanceLabel: {
    fontSize: 12,
    color: '#9ca3af',
    fontWeight: '500',
    marginBottom: 4,
  },
  balanceAmount: {
    fontSize: 28,
    fontWeight: '700',
    color: '#111827',
  },
  addressBox: {
    backgroundColor: '#ffffff',
    borderRadius: 12,
    padding: 14,
    width: '100%',
    marginBottom: 20,
  },
  addressLabel: {
    fontSize: 11,
    color: '#9ca3af',
    fontWeight: '500',
    marginBottom: 4,
  },
  addressText: {
    fontSize: 13,
    color: '#111827',
    fontFamily: 'monospace',
  },
  claimButton: {
    backgroundColor: '#22c55e',
    borderRadius: 14,
    paddingVertical: 16,
    width: '100%',
    alignItems: 'center',
    minHeight: 52,
    justifyContent: 'center',
    marginBottom: 20,
  },
  claimButtonDisabled: {
    backgroundColor: '#9ca3af',
  },
  claimButtonText: {
    color: '#ffffff',
    fontSize: 17,
    fontWeight: '700',
  },
  infoBox: {
    backgroundColor: '#ffffff',
    borderRadius: 12,
    padding: 16,
    width: '100%',
    marginBottom: 16,
  },
  infoRow: {
    flexDirection: 'row',
    justifyContent: 'space-between',
    paddingVertical: 8,
    borderBottomWidth: 1,
    borderBottomColor: '#f3f4f6',
  },
  infoLabel: {
    fontSize: 14,
    color: '#6b7280',
  },
  infoValue: {
    fontSize: 14,
    color: '#111827',
    fontWeight: '600',
  },
  disclaimer: {
    fontSize: 11,
    color: '#9ca3af',
    textAlign: 'center',
    lineHeight: 16,
  },
});

export default FaucetScreen;
