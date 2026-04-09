/**
 * SwapScreen — Token swap with quote fetching and execution
 *
 * Fetches real-time quotes from the DEX API, shows rate + price impact,
 * configurable slippage, biometric confirmation before execution.
 */

import React, { useState, useCallback, useEffect } from 'react';
import {
  View,
  Text,
  TextInput,
  TouchableOpacity,
  ScrollView,
  StyleSheet,
  Alert,
  ActivityIndicator,
  Modal,
  FlatList,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import * as LocalAuthentication from 'expo-local-authentication';
import { api } from '../utils/api';
import type { SwapQuote } from '../utils/api';
import { keystore } from '../utils/keystore';

const TOKENS = [
  { symbol: 'EVAP', name: 'EvaporChain', color: '#06b6d4' },
  { symbol: 'wETH', name: 'Wrapped Ether', color: '#627eea' },
  { symbol: 'wBTC', name: 'Wrapped Bitcoin', color: '#f7931a' },
  { symbol: 'USDC', name: 'USD Coin', color: '#2775ca' },
  { symbol: 'USDT', name: 'Tether', color: '#50af95' },
];

const SLIPPAGE_OPTIONS = [0.1, 0.5, 1.0, 3.0];

const SwapScreen: React.FC = () => {
  const [fromToken, setFromToken] = useState('EVAP');
  const [toToken, setToToken] = useState('USDC');
  const [fromAmount, setFromAmount] = useState('');
  const [quote, setQuote] = useState<SwapQuote | null>(null);
  const [quoteLoading, setQuoteLoading] = useState(false);
  const [swapping, setSwapping] = useState(false);
  const [slippage, setSlippage] = useState(0.5);
  const [selectorVisible, setSelectorVisible] = useState<'from' | 'to' | null>(null);

  // Debounced quote fetching
  useEffect(() => {
    const amount = parseFloat(fromAmount);
    if (!fromAmount || isNaN(amount) || amount <= 0) {
      setQuote(null);
      return;
    }

    const timer = setTimeout(async () => {
      setQuoteLoading(true);
      try {
        const q = await api.getSwapQuote(fromToken, toToken, amount);
        setQuote(q);
      } catch {
        setQuote(null);
      } finally {
        setQuoteLoading(false);
      }
    }, 500);

    return () => clearTimeout(timer);
  }, [fromAmount, fromToken, toToken]);

  const handleSwapDirection = () => {
    const prevFrom = fromToken;
    setFromToken(toToken);
    setToToken(prevFrom);
    setFromAmount('');
    setQuote(null);
  };

  const handleSelectToken = (symbol: string) => {
    if (selectorVisible === 'from') {
      if (symbol === toToken) setToToken(fromToken);
      setFromToken(symbol);
    } else {
      if (symbol === fromToken) setFromToken(toToken);
      setToToken(symbol);
    }
    setSelectorVisible(null);
    setQuote(null);
  };

  const handleMax = async () => {
    try {
      const address = await keystore.getAddress();
      if (!address) return;
      if (fromToken === 'EVAP') {
        const bal = await api.getBalance(address);
        setFromAmount(String(bal.balance));
      }
    } catch {
      // Keep existing
    }
  };

  const handleSwap = useCallback(async () => {
    if (!quote) return;

    const auth = await LocalAuthentication.authenticateAsync({
      promptMessage: 'Confirm Swap',
      cancelLabel: 'Cancel',
    });
    if (!auth.success) return;

    setSwapping(true);
    try {
      const amount = parseFloat(fromAmount);
      const result = await api.executeSwap(fromToken, toToken, amount, slippage);
      if (result.success) {
        Alert.alert(
          'Swap Complete',
          `Swapped ${fromAmount} ${fromToken} for ~${quote.amountOut.toFixed(4)} ${toToken}.\n${result.tx_hash ? `Hash: ${result.tx_hash.slice(0, 16)}...` : ''}`
        );
        setFromAmount('');
        setQuote(null);
      } else {
        Alert.alert('Swap Failed', result.message);
      }
    } catch {
      Alert.alert('Error', 'Swap transaction failed. Please try again.');
    } finally {
      setSwapping(false);
    }
  }, [quote, fromAmount, fromToken, toToken, slippage]);

  const getTokenColor = (symbol: string): string =>
    TOKENS.find((t) => t.symbol === symbol)?.color || '#6b7280';

  const getPriceImpactColor = (impact: number): string => {
    if (impact < 1) return '#22c55e';
    if (impact < 3) return '#f59e0b';
    return '#ef4444';
  };

  return (
    <SafeAreaView style={styles.container} edges={['bottom']}>
      <ScrollView contentContainerStyle={styles.content} keyboardShouldPersistTaps="handled">
        {/* From Token */}
        <View style={styles.tokenCard}>
          <View style={styles.tokenHeader}>
            <Text style={styles.tokenLabel}>From</Text>
            {fromToken === 'EVAP' && (
              <TouchableOpacity onPress={handleMax} activeOpacity={0.7}>
                <Text style={styles.maxText}>MAX</Text>
              </TouchableOpacity>
            )}
          </View>
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
              onPress={() => setSelectorVisible('from')}
              activeOpacity={0.7}
            >
              <View style={[styles.tokenDot, { backgroundColor: getTokenColor(fromToken) }]} />
              <Text style={styles.tokenName}>{fromToken}</Text>
              <Text style={styles.tokenChevron}>v</Text>
            </TouchableOpacity>
          </View>
        </View>

        {/* Swap Direction */}
        <TouchableOpacity
          style={styles.swapDirectionButton}
          onPress={handleSwapDirection}
          activeOpacity={0.7}
        >
          <Text style={styles.swapDirectionIcon}>{'<>'}</Text>
        </TouchableOpacity>

        {/* To Token */}
        <View style={styles.tokenCard}>
          <Text style={styles.tokenLabel}>To (estimated)</Text>
          <View style={styles.tokenInputRow}>
            <Text style={styles.estimatedAmount}>
              {quoteLoading ? '...' : quote ? quote.amountOut.toFixed(4) : '0.00'}
            </Text>
            <TouchableOpacity
              style={styles.tokenSelector}
              onPress={() => setSelectorVisible('to')}
              activeOpacity={0.7}
            >
              <View style={[styles.tokenDot, { backgroundColor: getTokenColor(toToken) }]} />
              <Text style={styles.tokenName}>{toToken}</Text>
              <Text style={styles.tokenChevron}>v</Text>
            </TouchableOpacity>
          </View>
        </View>

        {/* Quote Details */}
        {quote && (
          <View style={styles.quoteCard}>
            <View style={styles.quoteRow}>
              <Text style={styles.quoteLabel}>Rate</Text>
              <Text style={styles.quoteValue}>
                1 {fromToken} = {quote.rate.toFixed(6)} {toToken}
              </Text>
            </View>
            <View style={styles.quoteRow}>
              <Text style={styles.quoteLabel}>Price Impact</Text>
              <Text style={[styles.quoteValue, { color: getPriceImpactColor(quote.priceImpact) }]}>
                {quote.priceImpact.toFixed(2)}%
              </Text>
            </View>
            <View style={styles.quoteRow}>
              <Text style={styles.quoteLabel}>Min. Received</Text>
              <Text style={styles.quoteValue}>
                {(quote.amountOut * (1 - slippage / 100)).toFixed(4)} {toToken}
              </Text>
            </View>
          </View>
        )}

        {/* Slippage */}
        <View style={styles.slippageSection}>
          <Text style={styles.slippageLabel}>Slippage Tolerance</Text>
          <View style={styles.slippageRow}>
            {SLIPPAGE_OPTIONS.map((opt) => (
              <TouchableOpacity
                key={opt}
                style={[styles.slippageOption, slippage === opt && styles.slippageOptionActive]}
                onPress={() => setSlippage(opt)}
                activeOpacity={0.7}
              >
                <Text style={[styles.slippageText, slippage === opt && styles.slippageTextActive]}>
                  {opt}%
                </Text>
              </TouchableOpacity>
            ))}
          </View>
        </View>

        {/* Swap Button */}
        <TouchableOpacity
          style={[
            styles.swapButton,
            (!quote || swapping) && styles.swapButtonDisabled,
          ]}
          onPress={handleSwap}
          disabled={!quote || swapping}
          activeOpacity={0.7}
        >
          {swapping ? (
            <ActivityIndicator color="#ffffff" />
          ) : (
            <Text style={styles.swapButtonText}>
              {!fromAmount ? 'Enter Amount' : quoteLoading ? 'Fetching Quote...' : !quote ? 'No Quote Available' : 'Swap'}
            </Text>
          )}
        </TouchableOpacity>

        <Text style={styles.disclaimer}>
          Swap powered by EvaporChain DEX. Energy fees apply.
        </Text>
      </ScrollView>

      {/* Token Selector Modal */}
      <Modal visible={selectorVisible !== null} transparent animationType="slide">
        <View style={styles.modalOverlay}>
          <View style={styles.modalCard}>
            <View style={styles.modalHeader}>
              <Text style={styles.modalTitle}>Select Token</Text>
              <TouchableOpacity onPress={() => setSelectorVisible(null)} activeOpacity={0.7}>
                <Text style={styles.modalClose}>Close</Text>
              </TouchableOpacity>
            </View>
            <FlatList
              data={TOKENS}
              keyExtractor={(item) => item.symbol}
              renderItem={({ item }) => (
                <TouchableOpacity
                  style={styles.tokenOption}
                  onPress={() => handleSelectToken(item.symbol)}
                  activeOpacity={0.7}
                >
                  <View style={[styles.tokenOptionDot, { backgroundColor: item.color }]} />
                  <View style={styles.tokenOptionInfo}>
                    <Text style={styles.tokenOptionSymbol}>{item.symbol}</Text>
                    <Text style={styles.tokenOptionName}>{item.name}</Text>
                  </View>
                </TouchableOpacity>
              )}
            />
          </View>
        </View>
      </Modal>
    </SafeAreaView>
  );
};

const styles = StyleSheet.create({
  container: {
    flex: 1,
    backgroundColor: '#f9fafb',
  },
  content: {
    padding: 16,
    paddingBottom: 40,
  },
  tokenCard: {
    backgroundColor: '#ffffff',
    borderRadius: 14,
    padding: 16,
  },
  tokenHeader: {
    flexDirection: 'row',
    justifyContent: 'space-between',
    alignItems: 'center',
    marginBottom: 8,
  },
  tokenLabel: {
    fontSize: 13,
    color: '#6b7280',
    fontWeight: '500',
  },
  maxText: {
    fontSize: 13,
    fontWeight: '700',
    color: '#8b5cf6',
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
  estimatedAmount: {
    flex: 1,
    fontSize: 28,
    fontWeight: '600',
    color: '#6b7280',
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
  tokenDot: {
    width: 8,
    height: 8,
    borderRadius: 4,
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
  quoteCard: {
    backgroundColor: '#ffffff',
    borderRadius: 12,
    padding: 16,
    marginTop: 16,
  },
  quoteRow: {
    flexDirection: 'row',
    justifyContent: 'space-between',
    paddingVertical: 8,
    borderBottomWidth: 1,
    borderBottomColor: '#f3f4f6',
  },
  quoteLabel: {
    fontSize: 14,
    color: '#6b7280',
  },
  quoteValue: {
    fontSize: 14,
    color: '#111827',
    fontWeight: '500',
  },
  slippageSection: {
    marginTop: 16,
  },
  slippageLabel: {
    fontSize: 13,
    fontWeight: '600',
    color: '#6b7280',
    marginBottom: 8,
  },
  slippageRow: {
    flexDirection: 'row',
    gap: 8,
  },
  slippageOption: {
    flex: 1,
    backgroundColor: '#ffffff',
    borderRadius: 10,
    paddingVertical: 10,
    alignItems: 'center',
    borderWidth: 2,
    borderColor: '#e5e7eb',
  },
  slippageOptionActive: {
    borderColor: '#06b6d4',
    backgroundColor: '#ecfeff',
  },
  slippageText: {
    fontSize: 14,
    fontWeight: '600',
    color: '#6b7280',
  },
  slippageTextActive: {
    color: '#06b6d4',
  },
  swapButton: {
    backgroundColor: '#22c55e',
    borderRadius: 14,
    paddingVertical: 16,
    alignItems: 'center',
    marginTop: 20,
    minHeight: 52,
    justifyContent: 'center',
  },
  swapButtonDisabled: {
    backgroundColor: '#9ca3af',
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
  },
  // Token Selector Modal
  modalOverlay: {
    flex: 1,
    backgroundColor: 'rgba(0,0,0,0.5)',
    justifyContent: 'flex-end',
  },
  modalCard: {
    backgroundColor: '#ffffff',
    borderTopLeftRadius: 20,
    borderTopRightRadius: 20,
    maxHeight: '60%',
    paddingBottom: 32,
  },
  modalHeader: {
    flexDirection: 'row',
    justifyContent: 'space-between',
    alignItems: 'center',
    padding: 20,
    borderBottomWidth: 1,
    borderBottomColor: '#f3f4f6',
  },
  modalTitle: {
    fontSize: 17,
    fontWeight: '600',
    color: '#111827',
  },
  modalClose: {
    fontSize: 16,
    fontWeight: '600',
    color: '#06b6d4',
  },
  tokenOption: {
    flexDirection: 'row',
    alignItems: 'center',
    paddingHorizontal: 20,
    paddingVertical: 14,
    borderBottomWidth: 1,
    borderBottomColor: '#f9fafb',
  },
  tokenOptionDot: {
    width: 12,
    height: 12,
    borderRadius: 6,
    marginRight: 14,
  },
  tokenOptionInfo: {},
  tokenOptionSymbol: {
    fontSize: 16,
    fontWeight: '600',
    color: '#111827',
  },
  tokenOptionName: {
    fontSize: 13,
    color: '#9ca3af',
    marginTop: 1,
  },
});

export default SwapScreen;
