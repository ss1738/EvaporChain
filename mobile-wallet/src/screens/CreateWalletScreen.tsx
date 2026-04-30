/**
 * CreateWalletScreen — Generate new wallet with seed phrase backup
 *
 * Flow: Generate keypair -> Show seed phrase -> Confirm backup -> Set PIN -> Done
 */

import React, { useState, useEffect } from 'react';
import {
  View,
  Text,
  TextInput,
  TouchableOpacity,
  ScrollView,
  StyleSheet,
  Alert,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import type { NativeStackNavigationProp } from '@react-navigation/native-stack';
import type { RootStackParamList } from '../navigation/AppNavigator';
import { keystore } from '../utils/keystore';
import { generateWallet, type WalletKeys } from '../utils/keygen';

type Props = {
  navigation: NativeStackNavigationProp<RootStackParamList, 'CreateWallet'>;
};

type Step = 'generate' | 'backup' | 'confirm' | 'pin' | 'pin-confirm';

const CreateWalletScreen: React.FC<Props> = ({ navigation }) => {
  const [step, setStep] = useState<Step>('generate');
  const [wallet, setWallet] = useState<WalletKeys | null>(null);
  const [confirmWords, setConfirmWords] = useState<{ index: number; word: string }[]>([]);
  const [confirmInput, setConfirmInput] = useState<Record<number, string>>({});
  const [pin, setPin] = useState('');
  const [pinConfirm, setPinConfirm] = useState('');
  const [pinError, setPinError] = useState('');

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const keys = await generateWallet();
        if (cancelled) return;
        setWallet(keys);

        // Pick 3 random words for confirmation
        const words = keys.seedPhrase.split(' ');
        const indices = new Set<number>();
        while (indices.size < 3) {
          indices.add(Math.floor(Math.random() * words.length));
        }
        setConfirmWords(
          Array.from(indices)
            .sort((a, b) => a - b)
            .map((i) => ({ index: i, word: words[i] as string }))
        );
      } catch (err) {
        if (cancelled) return;
        const msg = err instanceof Error ? err.message : 'Failed to generate wallet';
        Alert.alert('Error', msg);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const handleBackupDone = () => {
    setStep('confirm');
  };

  const handleConfirmWords = () => {
    for (const { index, word } of confirmWords) {
      if (confirmInput[index]?.trim().toLowerCase() !== word.toLowerCase()) {
        Alert.alert('Incorrect', `Word #${index + 1} is incorrect. Please check your backup.`);
        return;
      }
    }
    setStep('pin');
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

    if (!wallet) return;

    await keystore.createWallet(
      wallet.privateKey,
      wallet.publicKey,
      wallet.address,
      wallet.seedPhrase,
      pin
    );

    navigation.replace('Unlock');
  };

  if (!wallet) return null;

  const seedWords = wallet.seedPhrase.split(' ');

  return (
    <SafeAreaView style={styles.container} edges={['bottom']}>
      <ScrollView contentContainerStyle={styles.scroll} keyboardShouldPersistTaps="handled">
        {step === 'generate' && (
          <>
            <Text style={styles.heading}>Your New Wallet</Text>
            <Text style={styles.description}>
              Your wallet has been generated. Your address is:
            </Text>
            <View style={styles.addressBox}>
              <Text style={styles.addressText} selectable>
                {wallet.address}
              </Text>
            </View>
            <Text style={styles.description}>
              Next, you will be shown your 24-word seed phrase. This is the only way
              to recover your wallet. Write it down and keep it safe.
            </Text>
            <TouchableOpacity
              style={styles.primaryButton}
              onPress={() => setStep('backup')}
              activeOpacity={0.7}
            >
              <Text style={styles.primaryButtonText}>Show Seed Phrase</Text>
            </TouchableOpacity>
          </>
        )}

        {step === 'backup' && (
          <>
            <Text style={styles.heading}>Backup Seed Phrase</Text>
            <Text style={styles.warningText}>
              Write down these 24 words in order. Never share them with anyone.
            </Text>
            <View style={styles.seedGrid}>
              {seedWords.map((word, i) => (
                <View key={i} style={styles.seedWord}>
                  <Text style={styles.seedIndex}>{i + 1}</Text>
                  <Text style={styles.seedText}>{word}</Text>
                </View>
              ))}
            </View>
            <TouchableOpacity
              style={styles.primaryButton}
              onPress={handleBackupDone}
              activeOpacity={0.7}
            >
              <Text style={styles.primaryButtonText}>I have written it down</Text>
            </TouchableOpacity>
          </>
        )}

        {step === 'confirm' && (
          <>
            <Text style={styles.heading}>Verify Backup</Text>
            <Text style={styles.description}>
              Enter the following words from your seed phrase to confirm your backup.
            </Text>
            {confirmWords.map(({ index }) => (
              <View key={index} style={styles.confirmField}>
                <Text style={styles.confirmLabel}>Word #{index + 1}</Text>
                <TextInput
                  style={styles.confirmInput}
                  value={confirmInput[index] || ''}
                  onChangeText={(text) =>
                    setConfirmInput((prev) => ({ ...prev, [index]: text }))
                  }
                  autoCapitalize="none"
                  autoCorrect={false}
                  placeholder="Enter word"
                  placeholderTextColor="#9ca3af"
                />
              </View>
            ))}
            <TouchableOpacity
              style={styles.primaryButton}
              onPress={handleConfirmWords}
              activeOpacity={0.7}
            >
              <Text style={styles.primaryButtonText}>Verify</Text>
            </TouchableOpacity>
          </>
        )}

        {step === 'pin' && (
          <>
            <Text style={styles.heading}>Set Your PIN</Text>
            <Text style={styles.description}>
              Choose a 6-digit PIN to secure your wallet.
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
              disabled={pinConfirm.length < 6}
              activeOpacity={0.7}
            >
              <Text style={styles.primaryButtonText}>Create Wallet</Text>
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
    marginBottom: 20,
  },
  warningText: {
    fontSize: 14,
    color: '#dc2626',
    fontWeight: '500',
    lineHeight: 20,
    marginBottom: 20,
    backgroundColor: '#fef2f2',
    padding: 12,
    borderRadius: 10,
  },
  addressBox: {
    backgroundColor: '#ffffff',
    borderRadius: 12,
    padding: 16,
    marginBottom: 20,
  },
  addressText: {
    fontSize: 13,
    color: '#111827',
    fontWeight: '500',
    fontFamily: 'monospace',
    lineHeight: 20,
  },
  seedGrid: {
    flexDirection: 'row',
    flexWrap: 'wrap',
    gap: 8,
    marginBottom: 24,
  },
  seedWord: {
    flexDirection: 'row',
    alignItems: 'center',
    backgroundColor: '#ffffff',
    borderRadius: 8,
    paddingHorizontal: 10,
    paddingVertical: 8,
    width: '31%',
    gap: 6,
  },
  seedIndex: {
    fontSize: 10,
    color: '#9ca3af',
    fontWeight: '600',
    minWidth: 16,
  },
  seedText: {
    fontSize: 13,
    color: '#111827',
    fontWeight: '500',
  },
  confirmField: {
    marginBottom: 16,
  },
  confirmLabel: {
    fontSize: 14,
    fontWeight: '600',
    color: '#374151',
    marginBottom: 6,
  },
  confirmInput: {
    backgroundColor: '#ffffff',
    borderWidth: 1,
    borderColor: '#e5e7eb',
    borderRadius: 12,
    paddingHorizontal: 16,
    paddingVertical: 14,
    fontSize: 16,
    color: '#111827',
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
    marginBottom: 12,
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

export default CreateWalletScreen;
