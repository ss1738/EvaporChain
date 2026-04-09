/**
 * ImportWalletScreen — Recover wallet from existing seed phrase
 *
 * Flow: Enter 24 words -> Set PIN -> Confirm PIN -> Done
 */

import React, { useState } from 'react';
import {
  View,
  Text,
  TextInput,
  TouchableOpacity,
  ScrollView,
  StyleSheet,
  Alert,
  ActivityIndicator,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import type { NativeStackNavigationProp } from '@react-navigation/native-stack';
import type { RootStackParamList } from '../navigation/AppNavigator';
import { keystore } from '../utils/keystore';
import { walletFromSeed } from '../utils/keygen';

type Props = {
  navigation: NativeStackNavigationProp<RootStackParamList, 'ImportWallet'>;
};

type Step = 'seed' | 'pin' | 'pin-confirm';

const ImportWalletScreen: React.FC<Props> = ({ navigation }) => {
  const [step, setStep] = useState<Step>('seed');
  const [seedInput, setSeedInput] = useState('');
  const [seedError, setSeedError] = useState('');
  const [recoveredAddress, setRecoveredAddress] = useState('');
  const [pin, setPin] = useState('');
  const [pinConfirm, setPinConfirm] = useState('');
  const [pinError, setPinError] = useState('');
  const [importing, setImporting] = useState(false);

  const handleValidateSeed = () => {
    const words = seedInput.trim().toLowerCase().split(/\s+/);
    if (words.length !== 24) {
      setSeedError(`Expected 24 words, got ${words.length}`);
      return;
    }

    try {
      const wallet = walletFromSeed(seedInput);
      setRecoveredAddress(wallet.address);
      setSeedError('');
      setStep('pin');
    } catch (err) {
      setSeedError('Invalid seed phrase. Please check your words.');
    }
  };

  const handleSetPin = () => {
    if (pin.length !== 6) {
      setPinError('PIN must be 6 digits');
      return;
    }
    setPinError('');
    setStep('pin-confirm');
  };

  const handleConfirmPin = async () => {
    if (pinConfirm !== pin) {
      setPinError('PINs do not match');
      setPinConfirm('');
      return;
    }

    setImporting(true);
    try {
      const wallet = walletFromSeed(seedInput);
      await keystore.createWallet(
        wallet.privateKey,
        wallet.publicKey,
        wallet.address,
        wallet.seedPhrase,
        pin
      );
      navigation.replace('Unlock');
    } catch {
      Alert.alert('Error', 'Failed to import wallet. Please try again.');
    } finally {
      setImporting(false);
    }
  };

  return (
    <SafeAreaView style={styles.container} edges={['bottom']}>
      <ScrollView contentContainerStyle={styles.scroll} keyboardShouldPersistTaps="handled">
        {step === 'seed' && (
          <>
            <Text style={styles.heading}>Import Wallet</Text>
            <Text style={styles.description}>
              Enter your 24-word seed phrase to recover your wallet.
              Separate words with spaces.
            </Text>
            <TextInput
              style={styles.seedInput}
              value={seedInput}
              onChangeText={(text) => {
                setSeedError('');
                setSeedInput(text);
              }}
              multiline
              numberOfLines={4}
              autoCapitalize="none"
              autoCorrect={false}
              placeholder="word1 word2 word3 ... word24"
              placeholderTextColor="#9ca3af"
              textAlignVertical="top"
            />
            {seedError ? <Text style={styles.errorText}>{seedError}</Text> : null}

            <Text style={styles.wordCount}>
              {seedInput.trim() ? seedInput.trim().split(/\s+/).length : 0} / 24 words
            </Text>

            <TouchableOpacity
              style={styles.primaryButton}
              onPress={handleValidateSeed}
              activeOpacity={0.7}
            >
              <Text style={styles.primaryButtonText}>Recover Wallet</Text>
            </TouchableOpacity>
          </>
        )}

        {step === 'pin' && (
          <>
            <Text style={styles.heading}>Set Your PIN</Text>
            <Text style={styles.description}>
              Wallet recovered! Address:
            </Text>
            <View style={styles.addressBox}>
              <Text style={styles.addressText} selectable>
                {recoveredAddress}
              </Text>
            </View>
            <Text style={styles.description}>
              Choose a 6-digit PIN to secure this wallet on your device.
            </Text>
            <TextInput
              style={styles.pinInput}
              value={pin}
              onChangeText={(text) => {
                setPinError('');
                setPin(text.replace(/[^0-9]/g, '').slice(0, 6));
              }}
              keyboardType="number-pad"
              secureTextEntry
              maxLength={6}
              placeholder="6-digit PIN"
              placeholderTextColor="#9ca3af"
              autoFocus
            />
            {pinError ? <Text style={styles.errorText}>{pinError}</Text> : null}
            <TouchableOpacity
              style={[styles.primaryButton, pin.length < 6 && styles.buttonDisabled]}
              onPress={handleSetPin}
              disabled={pin.length < 6}
              activeOpacity={0.7}
            >
              <Text style={styles.primaryButtonText}>Continue</Text>
            </TouchableOpacity>
          </>
        )}

        {step === 'pin-confirm' && (
          <>
            <Text style={styles.heading}>Confirm PIN</Text>
            <Text style={styles.description}>
              Enter your PIN again to confirm.
            </Text>
            <TextInput
              style={styles.pinInput}
              value={pinConfirm}
              onChangeText={(text) => {
                setPinError('');
                setPinConfirm(text.replace(/[^0-9]/g, '').slice(0, 6));
              }}
              keyboardType="number-pad"
              secureTextEntry
              maxLength={6}
              placeholder="Confirm PIN"
              placeholderTextColor="#9ca3af"
              autoFocus
            />
            {pinError ? <Text style={styles.errorText}>{pinError}</Text> : null}
            <TouchableOpacity
              style={[styles.primaryButton, pinConfirm.length < 6 && styles.buttonDisabled]}
              onPress={handleConfirmPin}
              disabled={pinConfirm.length < 6 || importing}
              activeOpacity={0.7}
            >
              {importing ? (
                <ActivityIndicator color="#ffffff" />
              ) : (
                <Text style={styles.primaryButtonText}>Import Wallet</Text>
              )}
            </TouchableOpacity>
          </>
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
  scroll: {
    padding: 24,
    paddingBottom: 40,
  },
  heading: {
    fontSize: 24,
    fontWeight: '700',
    color: '#111827',
    marginBottom: 12,
  },
  description: {
    fontSize: 15,
    color: '#6b7280',
    lineHeight: 22,
    marginBottom: 16,
  },
  seedInput: {
    backgroundColor: '#ffffff',
    borderWidth: 1,
    borderColor: '#e5e7eb',
    borderRadius: 12,
    paddingHorizontal: 16,
    paddingVertical: 14,
    fontSize: 15,
    color: '#111827',
    lineHeight: 22,
    minHeight: 120,
  },
  wordCount: {
    fontSize: 13,
    color: '#9ca3af',
    textAlign: 'right',
    marginTop: 6,
    marginBottom: 16,
  },
  addressBox: {
    backgroundColor: '#ffffff',
    borderRadius: 12,
    padding: 16,
    marginBottom: 16,
  },
  addressText: {
    fontSize: 13,
    color: '#111827',
    fontWeight: '500',
    fontFamily: 'monospace',
    lineHeight: 20,
  },
  pinInput: {
    backgroundColor: '#ffffff',
    borderRadius: 12,
    paddingHorizontal: 16,
    paddingVertical: 16,
    fontSize: 28,
    fontWeight: '600',
    textAlign: 'center',
    color: '#111827',
    letterSpacing: 8,
    marginBottom: 16,
  },
  errorText: {
    color: '#ef4444',
    fontSize: 13,
    textAlign: 'center',
    marginTop: 8,
    marginBottom: 8,
  },
  primaryButton: {
    backgroundColor: '#06b6d4',
    borderRadius: 14,
    paddingVertical: 16,
    alignItems: 'center',
    minHeight: 52,
    justifyContent: 'center',
    marginTop: 8,
  },
  buttonDisabled: {
    opacity: 0.5,
  },
  primaryButtonText: {
    color: '#ffffff',
    fontSize: 17,
    fontWeight: '700',
  },
});

export default ImportWalletScreen;
