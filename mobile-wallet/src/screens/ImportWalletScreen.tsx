/**
 * ImportWalletScreen — Recover wallet from existing seed phrase + backup JSON
 *
 * Flow:
 *   Enter 24 words + paste backup JSON
 *     -> validate mnemonic (BIP-39 + EvaporChain checksum)
 *     -> decrypt MnemonicBackup envelope (AES-256-GCM, BLAKE3-derived key)
 *     -> recover real ML-DSA keypair
 *   -> Set PIN -> Confirm PIN -> Done
 *
 * Mnemonic-only "recovery" is intentionally rejected: ML-DSA does not
 * support deterministic seed-to-key derivation, so the encrypted backup
 * JSON produced at wallet creation is required.
 */

import React, { useState } from 'react';
import {
  Text,
  TextInput,
  TouchableOpacity,
  ScrollView,
  StyleSheet,
  Alert,
  ActivityIndicator,
  View,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import type { NativeStackNavigationProp } from '@react-navigation/native-stack';
import type { RootStackParamList } from '../navigation/AppNavigator';
import { keystore, type MnemonicBackup } from '../utils/keystore';
import { validateMnemonic } from '../utils/keygen';

type Props = {
  navigation: NativeStackNavigationProp<RootStackParamList, 'ImportWallet'>;
};

type Step = 'seed' | 'pin' | 'pin-confirm';

interface RecoveredKeys {
  publicKey: string;
  privateKey: string;
  address: string;
  accountIndex: number;
}

const ImportWalletScreen: React.FC<Props> = ({ navigation }) => {
  const [step, setStep] = useState<Step>('seed');
  const [seedInput, setSeedInput] = useState('');
  const [backupInput, setBackupInput] = useState('');
  const [seedError, setSeedError] = useState('');
  const [recovered, setRecovered] = useState<RecoveredKeys | null>(null);
  const [seedPhraseClean, setSeedPhraseClean] = useState('');
  const [pin, setPin] = useState('');
  const [pinConfirm, setPinConfirm] = useState('');
  const [pinError, setPinError] = useState('');
  const [importing, setImporting] = useState(false);

  const handleValidateSeed = () => {
    setSeedError('');
    const phrase = seedInput.trim().toLowerCase().replace(/\s+/g, ' ');
    const words = phrase.split(/\s+/).filter(Boolean);
    if (words.length !== 24) {
      setSeedError(`Expected 24 words, got ${words.length}`);
      return;
    }
    if (!validateMnemonic(phrase)) {
      setSeedError('Invalid mnemonic — checksum failed. Check your words.');
      return;
    }

    if (!backupInput.trim()) {
      setSeedError(
        'Backup JSON is required. ML-DSA cannot be re-derived from a mnemonic alone — paste the encrypted backup envelope you saved at wallet creation.'
      );
      return;
    }

    let parsed: MnemonicBackup;
    try {
      parsed = JSON.parse(backupInput.trim()) as MnemonicBackup;
    } catch {
      setSeedError('Backup JSON is not valid JSON.');
      return;
    }

    if (
      !parsed ||
      typeof parsed !== 'object' ||
      typeof parsed.encrypted_keypair !== 'string' ||
      typeof parsed.nonce !== 'string' ||
      typeof parsed.address !== 'string' ||
      typeof parsed.version !== 'number'
    ) {
      setSeedError('Backup JSON missing required fields.');
      return;
    }

    try {
      const result = keystore.importBackup(phrase, parsed);
      setRecovered(result);
      setSeedPhraseClean(phrase);
      setStep('pin');
    } catch (err) {
      const msg = err instanceof Error ? err.message : 'Failed to decrypt backup';
      setSeedError(msg);
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
    if (!recovered) {
      Alert.alert('Error', 'No recovered wallet — please restart the import.');
      return;
    }

    setImporting(true);
    try {
      await keystore.createWallet(
        recovered.privateKey,
        recovered.publicKey,
        recovered.address,
        seedPhraseClean,
        pin,
        recovered.accountIndex
      );
      navigation.replace('Unlock');
    } catch (err) {
      const msg = err instanceof Error ? err.message : 'Failed to import wallet';
      Alert.alert('Error', msg);
    } finally {
      setImporting(false);
    }
  };

  const wordCount = seedInput.trim() ? seedInput.trim().split(/\s+/).length : 0;

  return (
    <SafeAreaView style={styles.container} edges={['bottom']}>
      <ScrollView contentContainerStyle={styles.scroll} keyboardShouldPersistTaps="handled">
        {step === 'seed' && (
          <>
            <Text style={styles.heading}>Import Wallet</Text>
            <Text style={styles.description}>
              Paste your 24-word mnemonic and the encrypted backup JSON you saved
              when this wallet was created.  Both are required: ML-DSA keys
              cannot be re-derived from the words alone.
            </Text>

            <Text style={styles.label}>Mnemonic (24 words)</Text>
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
            <Text style={styles.wordCount}>{wordCount} / 24 words</Text>

            <Text style={styles.label}>Backup JSON</Text>
            <TextInput
              style={styles.backupInput}
              value={backupInput}
              onChangeText={(text) => {
                setSeedError('');
                setBackupInput(text);
              }}
              multiline
              numberOfLines={6}
              autoCapitalize="none"
              autoCorrect={false}
              placeholder='{"version":1,"account_index":0,"encrypted_keypair":"...","nonce":"...","address":"0x..."}'
              placeholderTextColor="#9ca3af"
              textAlignVertical="top"
            />

            {seedError ? <Text style={styles.errorText}>{seedError}</Text> : null}

            <TouchableOpacity
              style={styles.primaryButton}
              onPress={handleValidateSeed}
              activeOpacity={0.7}
            >
              <Text style={styles.primaryButtonText}>Recover Wallet</Text>
            </TouchableOpacity>
          </>
        )}

        {step === 'pin' && recovered && (
          <>
            <Text style={styles.heading}>Set Your PIN</Text>
            <Text style={styles.description}>Wallet recovered. Address:</Text>
            <View style={styles.addressBox}>
              <Text style={styles.addressText} selectable>
                {recovered.address}
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
            <Text style={styles.description}>Enter your PIN again to confirm.</Text>
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
              style={[
                styles.primaryButton,
                (pinConfirm.length < 6 || importing) && styles.buttonDisabled,
              ]}
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
  label: {
    fontSize: 13,
    fontWeight: '600',
    color: '#374151',
    marginBottom: 6,
    marginTop: 8,
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
    minHeight: 110,
  },
  backupInput: {
    backgroundColor: '#ffffff',
    borderWidth: 1,
    borderColor: '#e5e7eb',
    borderRadius: 12,
    paddingHorizontal: 16,
    paddingVertical: 14,
    fontSize: 13,
    color: '#111827',
    fontFamily: 'monospace',
    lineHeight: 18,
    minHeight: 140,
    marginBottom: 8,
  },
  wordCount: {
    fontSize: 13,
    color: '#9ca3af',
    textAlign: 'right',
    marginTop: 6,
    marginBottom: 8,
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
    marginTop: 12,
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
