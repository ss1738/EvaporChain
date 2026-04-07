/**
 * SwapScreen — Token swap interface
 *
 * Placeholder for future DEX integration on EvaporChain.
 */

import React, { useState } from 'react';
import {
  View,
  Text,
  TextInput,
  TouchableOpacity,
  StyleSheet,
  Alert,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';

const TOKENS = ['EVAP', 'wETH', 'wBTC', 'USDC'];

const SwapScreen: React.FC = () => {
  const [fromToken, setFromToken] = useState('EVAP');
  const [toToken, setToToken] = useState('USDC');
  const [fromAmount, setFromAmount] = useState('');
  const [toAmount, setToAmount] = useState('');

  const handleSwapDirection = () => {
    setFromToken(toToken);
    setToToken(fromToken);
    setFromAmount(toAmount);
    setToAmount(fromAmount);
  };

  const handleSwap = () => {
    Alert.alert(
      'Swap Coming Soon',
      'The EvaporChain DEX is under development. Swap functionality will be available once the DEX module launches on mainnet.'
    );
  };

  const selectToken = (position: 'from' | 'to') => {
    const current = position === 'from' ? fromToken : toToken;
    const other = position === 'from' ? toToken : fromToken;
    const available = TOKENS.filter((t) => t !== other);
    const next = available[(available.indexOf(current) + 1) % available.length];
    if (position === 'from') setFromToken(next);
    else setToToken(next);
  };

  return (
    <SafeAreaView style={styles.container} edges={['bottom']}>
      <View style={styles.content}>
        {/* From token */}
        <View style={styles.tokenCard}>
          <Text style={styles.tokenLabel}>From</Text>
          <View style={styles.tokenInputRow}>
            <TextInput
              style={styles.amountInput}
              placeholder="0.00"
              placeholderTextColor="#9ca3af"
              value={fromAmount}
              onChangeText={setFromAmount}
              keyboardType="decimal-pad"
            />
            <TouchableOpacity
              style={styles.tokenSelector}
              onPress={() => selectToken('from')}
              activeOpacity={0.7}
            >
              <Text style={styles.tokenName}>{fromToken}</Text>
              <Text style={styles.tokenChevron}>v</Text>
            </TouchableOpacity>
          </View>
        </View>

        {/* Swap direction button */}
        <TouchableOpacity
          style={styles.swapDirectionButton}
          onPress={handleSwapDirection}
          activeOpacity={0.7}
        >
          <Text style={styles.swapDirectionIcon}>{'<>'}</Text>
        </TouchableOpacity>

        {/* To token */}
        <View style={styles.tokenCard}>
          <Text style={styles.tokenLabel}>To</Text>
          <View style={styles.tokenInputRow}>
            <TextInput
              style={styles.amountInput}
              placeholder="0.00"
              placeholderTextColor="#9ca3af"
              value={toAmount}
              onChangeText={setToAmount}
              keyboardType="decimal-pad"
            />
            <TouchableOpacity
              style={styles.tokenSelector}
              onPress={() => selectToken('to')}
              activeOpacity={0.7}
            >
              <Text style={styles.tokenName}>{toToken}</Text>
              <Text style={styles.tokenChevron}>v</Text>
            </TouchableOpacity>
          </View>
        </View>

        {/* Rate info */}
        <View style={styles.rateInfo}>
          <Text style={styles.rateText}>
            1 {fromToken} = --- {toToken}
          </Text>
          <Text style={styles.rateSubtext}>Rate unavailable — DEX not yet live</Text>
        </View>

        {/* Swap button */}
        <TouchableOpacity
          style={styles.swapButton}
          onPress={handleSwap}
          activeOpacity={0.7}
        >
          <Text style={styles.swapButtonText}>Swap</Text>
        </TouchableOpacity>

        <Text style={styles.disclaimer}>
          Swap powered by EvaporChain DEX. Slippage tolerance: 0.5%.
          Energy fees apply to swap transactions.
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
    padding: 16,
  },
  tokenCard: {
    backgroundColor: '#ffffff',
    borderRadius: 14,
    padding: 16,
  },
  tokenLabel: {
    fontSize: 13,
    color: '#6b7280',
    fontWeight: '500',
    marginBottom: 8,
  },
  tokenInputRow: {
    flexDirection: 'row',
    alignItems: 'center',
  },
  amountInput: {
    flex: 1,
    fontSize: 28,
    fontWeight: '600',
    color: '#111827',
    paddingVertical: 4,
  },
  tokenSelector: {
    flexDirection: 'row',
    alignItems: 'center',
    backgroundColor: '#f3f4f6',
    paddingHorizontal: 14,
    paddingVertical: 10,
    borderRadius: 12,
    gap: 6,
    minHeight: 44,
  },
  tokenName: {
    fontSize: 16,
    fontWeight: '700',
    color: '#111827',
  },
  tokenChevron: {
    fontSize: 12,
    color: '#6b7280',
  },
  swapDirectionButton: {
    alignSelf: 'center',
    width: 44,
    height: 44,
    borderRadius: 22,
    backgroundColor: '#06b6d4',
    alignItems: 'center',
    justifyContent: 'center',
    marginVertical: -12,
    zIndex: 10,
    shadowColor: '#06b6d4',
    shadowOffset: { width: 0, height: 2 },
    shadowOpacity: 0.3,
    shadowRadius: 4,
    elevation: 4,
  },
  swapDirectionIcon: {
    color: '#ffffff',
    fontSize: 16,
    fontWeight: '700',
  },
  rateInfo: {
    alignItems: 'center',
    marginTop: 20,
    marginBottom: 16,
  },
  rateText: {
    fontSize: 14,
    color: '#374151',
    fontWeight: '500',
  },
  rateSubtext: {
    fontSize: 12,
    color: '#9ca3af',
    marginTop: 4,
  },
  swapButton: {
    backgroundColor: '#22c55e',
    borderRadius: 14,
    paddingVertical: 16,
    alignItems: 'center',
    minHeight: 52,
    justifyContent: 'center',
  },
  swapButtonText: {
    color: '#ffffff',
    fontSize: 17,
    fontWeight: '700',
  },
  disclaimer: {
    fontSize: 11,
    color: '#9ca3af',
    textAlign: 'center',
    marginTop: 16,
    lineHeight: 16,
    paddingHorizontal: 16,
  },
});

export default SwapScreen;
