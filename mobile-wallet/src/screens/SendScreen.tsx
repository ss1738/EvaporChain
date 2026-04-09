/**
 * SendScreen — Send EVAP tokens
 *
 * Features: recipient address input, QR scan, amount with MAX,
 * transaction simulation preview, biometric/PIN confirmation.
 */

import React, { useState, useCallback } from 'react';
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
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { CameraView, useCameraPermissions } from 'expo-camera';
import * as LocalAuthentication from 'expo-local-authentication';
import type { NativeStackNavigationProp } from '@react-navigation/native-stack';
import type { RouteProp } from '@react-navigation/native';
import type { RootStackParamList } from '../navigation/AppNavigator';
import { api } from '../utils/api';
import { keystore } from '../utils/keystore';

type Props = {
  navigation: NativeStackNavigationProp<RootStackParamList, 'Send'>;
  route: RouteProp<RootStackParamList, 'Send'>;
};

const SendScreen: React.FC<Props> = ({ navigation, route }) => {
  const [recipient, setRecipient] = useState(route.params?.prefillAddress || '');
  const [amount, setAmount] = useState('');
  const [simulation, setSimulation] = useState<{ estimatedFee: string; estimatedEnergyCost: number; willSucceed: boolean; reason?: string } | null>(null);
  const [simulating, setSimulating] = useState(false);
  const [sending, setSending] = useState(false);
  const [scannerVisible, setScannerVisible] = useState(false);
  const [permission, requestPermission] = useCameraPermissions();

  const handleScanQR = async () => {
    if (!permission?.granted) {
      const result = await requestPermission();
      if (!result.granted) {
        Alert.alert('Permission Required', 'Camera access is needed to scan QR codes.');
        return;
      }
    }
    setScannerVisible(true);
  };

  const handleBarCodeScanned = (result: { data: string }) => {
    setScannerVisible(false);
    const address = result.data.replace(/^evaporchain:/i, '').trim();
    setRecipient(address);
  };

  const handleMax = async () => {
    try {
      const address = await keystore.getAddress();
      if (!address) return;
      const balance = await api.getBalance(address);
      setAmount(String(balance.balance));
    } catch {
      Alert.alert('Error', 'Could not fetch balance.');
    }
  };

  const handleSimulate = useCallback(async () => {
    if (!recipient.trim() || !amount.trim()) {
      Alert.alert('Missing Fields', 'Enter a recipient address and amount.');
      return;
    }

    setSimulating(true);
    try {
      const address = await keystore.getAddress();
      if (!address) return;
      const balance = await api.getBalance(address);
      const parsedAmount = parseFloat(amount.trim());
      const willSucceed = balance.balance >= parsedAmount && parsedAmount > 0;
      setSimulation({
        estimatedFee: '0.001',
        estimatedEnergyCost: Math.ceil(parsedAmount * 0.01),
        willSucceed,
        reason: willSucceed ? undefined : 'Insufficient balance',
      });
    } catch {
      Alert.alert('Simulation Error', 'Could not simulate the transaction.');
    } finally {
      setSimulating(false);
    }
  }, [recipient, amount]);

  const handleConfirmSend = async () => {
    if (!simulation?.willSucceed) {
      Alert.alert('Cannot Send', simulation?.reason || 'Transaction would fail.');
      return;
    }

    // Biometric / PIN confirmation
    const authResult = await LocalAuthentication.authenticateAsync({
      promptMessage: 'Confirm Transaction',
      cancelLabel: 'Cancel',
    });

    if (!authResult.success) return;

    setSending(true);
    try {
      const address = await keystore.getAddress();
      if (!address) throw new Error('No wallet address');

      const balance = await api.getBalance(address);
      const parsedAmount = parseFloat(amount.trim());

      // Send transfer via REST API — node signs with its keypair for mobile
      // Future: bundle Dilithium3 JS lib for client-side ML-DSA signing
      const result = await api.transfer(address, recipient.trim(), parsedAmount, balance.nonce);
      if (result.success) {
        Alert.alert('Sent!', `${parsedAmount} EVAP sent. ${result.tx_hash ? `Hash: ${result.tx_hash.slice(0, 16)}...` : ''}`, [
          { text: 'OK', onPress: () => navigation.goBack() },
        ]);
      } else {
        Alert.alert('Send Failed', result.message);
      }
    } catch {
      Alert.alert('Send Failed', 'The transaction could not be submitted.');
    } finally {
      setSending(false);
    }
  };

  return (
    <SafeAreaView style={styles.container} edges={['bottom']}>
      <ScrollView contentContainerStyle={styles.scroll} keyboardShouldPersistTaps="handled">
        {/* Recipient */}
        <View style={styles.field}>
          <Text style={styles.label}>Recipient Address</Text>
          <View style={styles.inputRow}>
            <TextInput
              style={styles.input}
              placeholder="evap1..."
              placeholderTextColor="#9ca3af"
              value={recipient}
              onChangeText={setRecipient}
              autoCapitalize="none"
              autoCorrect={false}
            />
            <TouchableOpacity style={styles.qrButton} onPress={handleScanQR} activeOpacity={0.7}>
              <Text style={styles.qrButtonText}>QR</Text>
            </TouchableOpacity>
          </View>
        </View>

        {/* Amount */}
        <View style={styles.field}>
          <Text style={styles.label}>Amount (EVAP)</Text>
          <View style={styles.inputRow}>
            <TextInput
              style={styles.input}
              placeholder="0.00"
              placeholderTextColor="#9ca3af"
              value={amount}
              onChangeText={setAmount}
              keyboardType="decimal-pad"
            />
            <TouchableOpacity style={styles.maxButton} onPress={handleMax} activeOpacity={0.7}>
              <Text style={styles.maxButtonText}>MAX</Text>
            </TouchableOpacity>
          </View>
        </View>

        {/* Simulate button */}
        <TouchableOpacity
          style={styles.simulateButton}
          onPress={handleSimulate}
          disabled={simulating}
          activeOpacity={0.7}
        >
          {simulating ? (
            <ActivityIndicator color="#06b6d4" />
          ) : (
            <Text style={styles.simulateButtonText}>Preview Transaction</Text>
          )}
        </TouchableOpacity>

        {/* Simulation preview */}
        {simulation && (
          <View style={styles.preview}>
            <Text style={styles.previewTitle}>Transaction Preview</Text>
            <View style={styles.previewRow}>
              <Text style={styles.previewLabel}>Amount</Text>
              <Text style={styles.previewValue}>{amount} EVAP</Text>
            </View>
            <View style={styles.previewRow}>
              <Text style={styles.previewLabel}>Network Fee</Text>
              <Text style={styles.previewValue}>{simulation.estimatedFee} EVAP</Text>
            </View>
            <View style={styles.previewRow}>
              <Text style={styles.previewLabel}>Energy Cost</Text>
              <Text style={styles.previewValue}>{simulation.estimatedEnergyCost} units</Text>
            </View>
            <View style={[styles.previewRow, styles.previewTotal]}>
              <Text style={styles.previewTotalLabel}>Total</Text>
              <Text style={styles.previewTotalValue}>
                {(parseFloat(amount) + parseFloat(simulation.estimatedFee)).toFixed(4)} EVAP
              </Text>
            </View>
            {!simulation.willSucceed && (
              <View style={styles.previewError}>
                <Text style={styles.previewErrorText}>{simulation.reason}</Text>
              </View>
            )}
          </View>
        )}

        {/* Confirm Send */}
        {simulation?.willSucceed && (
          <TouchableOpacity
            style={styles.sendButton}
            onPress={handleConfirmSend}
            disabled={sending}
            activeOpacity={0.7}
          >
            {sending ? (
              <ActivityIndicator color="#ffffff" />
            ) : (
              <Text style={styles.sendButtonText}>Confirm &amp; Send</Text>
            )}
          </TouchableOpacity>
        )}
      </ScrollView>

      {/* QR Scanner Modal */}
      <Modal visible={scannerVisible} animationType="slide">
        <SafeAreaView style={styles.scannerContainer}>
          <View style={styles.scannerHeader}>
            <Text style={styles.scannerTitle}>Scan QR Code</Text>
            <TouchableOpacity onPress={() => setScannerVisible(false)} activeOpacity={0.7}>
              <Text style={styles.scannerClose}>Close</Text>
            </TouchableOpacity>
          </View>
          <CameraView
            style={styles.camera}
            barcodeScannerSettings={{ barcodeTypes: ['qr'] }}
            onBarcodeScanned={handleBarCodeScanned}
          >
            <View style={styles.scannerOverlay}>
              <View style={styles.scannerFrame} />
              <Text style={styles.scannerHint}>
                Point at an EvaporChain address QR code
              </Text>
            </View>
          </CameraView>
        </SafeAreaView>
      </Modal>
    </SafeAreaView>
  );
};

const styles = StyleSheet.create({
  container: {
    flex: 1,
    backgroundColor: '#f9fafb',
  },
  scroll: {
    padding: 16,
    paddingBottom: 40,
  },
  field: {
    marginBottom: 20,
  },
  label: {
    fontSize: 14,
    fontWeight: '600',
    color: '#374151',
    marginBottom: 8,
  },
  inputRow: {
    flexDirection: 'row',
    gap: 8,
  },
  input: {
    flex: 1,
    backgroundColor: '#ffffff',
    borderWidth: 1,
    borderColor: '#e5e7eb',
    borderRadius: 12,
    paddingHorizontal: 16,
    paddingVertical: 14,
    fontSize: 16,
    color: '#111827',
  },
  qrButton: {
    width: 48,
    height: 48,
    borderRadius: 12,
    backgroundColor: '#06b6d4',
    alignItems: 'center',
    justifyContent: 'center',
    alignSelf: 'center',
  },
  qrButtonText: {
    color: '#ffffff',
    fontSize: 14,
    fontWeight: '700',
  },
  maxButton: {
    paddingHorizontal: 16,
    height: 48,
    borderRadius: 12,
    backgroundColor: '#8b5cf6',
    alignItems: 'center',
    justifyContent: 'center',
    alignSelf: 'center',
  },
  maxButtonText: {
    color: '#ffffff',
    fontSize: 13,
    fontWeight: '700',
  },
  simulateButton: {
    backgroundColor: '#ffffff',
    borderWidth: 1,
    borderColor: '#06b6d4',
    borderRadius: 12,
    paddingVertical: 14,
    alignItems: 'center',
    marginBottom: 16,
    minHeight: 48,
  },
  simulateButtonText: {
    color: '#06b6d4',
    fontSize: 15,
    fontWeight: '600',
  },
  preview: {
    backgroundColor: '#ffffff',
    borderRadius: 12,
    padding: 16,
    marginBottom: 16,
  },
  previewTitle: {
    fontSize: 14,
    fontWeight: '600',
    color: '#111827',
    marginBottom: 12,
  },
  previewRow: {
    flexDirection: 'row',
    justifyContent: 'space-between',
    paddingVertical: 8,
    borderBottomWidth: 1,
    borderBottomColor: '#f3f4f6',
  },
  previewLabel: {
    fontSize: 14,
    color: '#6b7280',
  },
  previewValue: {
    fontSize: 14,
    color: '#111827',
    fontWeight: '500',
  },
  previewTotal: {
    borderBottomWidth: 0,
    marginTop: 4,
  },
  previewTotalLabel: {
    fontSize: 15,
    color: '#111827',
    fontWeight: '700',
  },
  previewTotalValue: {
    fontSize: 15,
    color: '#111827',
    fontWeight: '700',
  },
  previewError: {
    backgroundColor: '#fef2f2',
    borderRadius: 8,
    padding: 12,
    marginTop: 12,
  },
  previewErrorText: {
    color: '#ef4444',
    fontSize: 13,
  },
  sendButton: {
    backgroundColor: '#22c55e',
    borderRadius: 12,
    paddingVertical: 16,
    alignItems: 'center',
    minHeight: 52,
  },
  sendButtonText: {
    color: '#ffffff',
    fontSize: 16,
    fontWeight: '700',
  },
  scannerContainer: {
    flex: 1,
    backgroundColor: '#000000',
  },
  scannerHeader: {
    flexDirection: 'row',
    justifyContent: 'space-between',
    alignItems: 'center',
    paddingHorizontal: 16,
    paddingVertical: 12,
    backgroundColor: '#000000',
  },
  scannerTitle: {
    fontSize: 17,
    fontWeight: '600',
    color: '#ffffff',
  },
  scannerClose: {
    fontSize: 16,
    fontWeight: '600',
    color: '#06b6d4',
  },
  camera: {
    flex: 1,
  },
  scannerOverlay: {
    flex: 1,
    alignItems: 'center',
    justifyContent: 'center',
  },
  scannerFrame: {
    width: 240,
    height: 240,
    borderWidth: 2,
    borderColor: '#06b6d4',
    borderRadius: 16,
  },
  scannerHint: {
    color: '#ffffff',
    fontSize: 14,
    marginTop: 20,
    textAlign: 'center',
    opacity: 0.8,
  },
});

export default SendScreen;
