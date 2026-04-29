/**
 * SettingsScreen — Wallet configuration and security
 *
 * Network selector, seed backup, keystore export, PIN change,
 * auto-lock timeout, push notification preferences.
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
  Switch,
  Modal,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import * as LocalAuthentication from 'expo-local-authentication';
import * as Clipboard from 'expo-clipboard';
import { useNavigation } from '@react-navigation/native';
import type { NativeStackNavigationProp } from '@react-navigation/native-stack';
import type { RootStackParamList } from '../navigation/AppNavigator';
import { api } from '../utils/api';
import { keystore } from '../utils/keystore';

const NETWORKS = ['testnet', 'mainnet'] as const;
const AUTO_LOCK_OPTIONS = [1, 5, 15, 30] as const;

type PinModalMode = 'export' | 'change-current' | 'change-new' | 'change-confirm' | null;

const SettingsScreen: React.FC = () => {
  const navigation = useNavigation<NativeStackNavigationProp<RootStackParamList>>();
  const [network, setNetwork] = useState<string>(api.getNetwork());
  const [autoLockMinutes, setAutoLockMinutes] = useState(5);
  const [pushEnabled, setPushEnabled] = useState(true);
  const [decayAlerts, setDecayAlerts] = useState(true);
  const [transferAlerts, setTransferAlerts] = useState(true);

  // PIN modal state
  const [pinModalMode, setPinModalMode] = useState<PinModalMode>(null);
  const [pinInput, setPinInput] = useState('');
  const [pinError, setPinError] = useState('');
  const [currentPinForChange, setCurrentPinForChange] = useState('');
  const [newPinForChange, setNewPinForChange] = useState('');

  useEffect(() => {
    loadSettings();
  }, []);

  const loadSettings = async () => {
    const timeout = await keystore.getAutoLockTimeout();
    setAutoLockMinutes(timeout);
  };

  const handleNetworkChange = (newNetwork: string) => {
    Alert.alert(
      'Switch Network',
      `Switch to ${newNetwork}? You will need to reconnect.`,
      [
        { text: 'Cancel', style: 'cancel' },
        {
          text: 'Switch',
          onPress: () => {
            api.setNetwork(newNetwork as 'testnet' | 'mainnet');
            setNetwork(newNetwork);
          },
        },
      ]
    );
  };

  const handleBackupSeed = async () => {
    const result = await LocalAuthentication.authenticateAsync({
      promptMessage: 'Authenticate to reveal seed phrase',
      cancelLabel: 'Cancel',
    });

    if (!result.success) return;

    const seed = await keystore.getSeedPhrase();
    if (!seed) {
      Alert.alert('Error', 'No seed phrase found.');
      return;
    }

    Alert.alert(
      'Seed Phrase',
      `Write this down and store it safely. Never share it.\n\n${seed}`,
      [{ text: 'I have saved it', style: 'destructive' }]
    );
  };

  const handleExportKeystore = () => {
    resetPinModal();
    setPinModalMode('export');
  };

  const handleChangePin = () => {
    resetPinModal();
    setPinModalMode('change-current');
  };

  const resetPinModal = () => {
    setPinInput('');
    setPinError('');
    setCurrentPinForChange('');
    setNewPinForChange('');
  };

  const closePinModal = () => {
    setPinModalMode(null);
    resetPinModal();
  };

  const handlePinSubmit = async () => {
    if (pinInput.length !== 6) {
      setPinError('PIN must be 6 digits');
      return;
    }

    if (pinModalMode === 'export') {
      const data = await keystore.exportKeystore(pinInput);
      if (!data) {
        setPinError('Incorrect PIN');
        setPinInput('');
        return;
      }
      closePinModal();
      await Clipboard.setStringAsync(data);
      Alert.alert('Exported', 'Keystore JSON copied to clipboard.');
    } else if (pinModalMode === 'change-current') {
      const valid = await keystore.verifyPin(pinInput);
      if (!valid) {
        setPinError('Incorrect PIN');
        setPinInput('');
        return;
      }
      setCurrentPinForChange(pinInput);
      setPinInput('');
      setPinError('');
      setPinModalMode('change-new');
    } else if (pinModalMode === 'change-new') {
      setNewPinForChange(pinInput);
      setPinInput('');
      setPinError('');
      setPinModalMode('change-confirm');
    } else if (pinModalMode === 'change-confirm') {
      if (pinInput !== newPinForChange) {
        setPinError('PINs do not match');
        setPinInput('');
        return;
      }
      const success = await keystore.changePin(currentPinForChange, pinInput);
      closePinModal();
      if (success) {
        Alert.alert('PIN Changed', 'Your wallet PIN has been updated.');
      } else {
        Alert.alert('Error', 'Could not change PIN. Please try again.');
      }
    }
  };

  const getPinModalTitle = (): string => {
    switch (pinModalMode) {
      case 'export': return 'Enter PIN to Export';
      case 'change-current': return 'Enter Current PIN';
      case 'change-new': return 'Enter New PIN';
      case 'change-confirm': return 'Confirm New PIN';
      default: return '';
    }
  };

  const handleDeleteWallet = () => {
    Alert.alert(
      'Delete Wallet',
      'This will permanently remove your wallet from this device. Make sure you have your seed phrase backed up. This cannot be undone.',
      [
        { text: 'Cancel', style: 'cancel' },
        {
          text: 'Delete',
          style: 'destructive',
          onPress: async () => {
            const auth = await LocalAuthentication.authenticateAsync({
              promptMessage: 'Confirm wallet deletion',
              cancelLabel: 'Cancel',
            });
            if (!auth.success) return;

            await keystore.deleteWallet();
            navigation.reset({ index: 0, routes: [{ name: 'Welcome' }] });
          },
        },
      ]
    );
  };

  const handleAutoLockChange = (minutes: number) => {
    setAutoLockMinutes(minutes);
    keystore.setAutoLockTimeout(minutes);
  };

  return (
    <SafeAreaView style={styles.container} edges={['bottom']}>
      <ScrollView contentContainerStyle={styles.scroll}>
        {/* Network */}
        <View style={styles.section}>
          <Text style={styles.sectionTitle}>Network</Text>
          <View style={styles.networkRow}>
            {NETWORKS.map((net) => (
              <TouchableOpacity
                key={net}
                style={[
                  styles.networkOption,
                  network === net && styles.networkOptionActive,
                ]}
                onPress={() => handleNetworkChange(net)}
                activeOpacity={0.7}
              >
                <View
                  style={[
                    styles.networkDot,
                    {
                      backgroundColor:
                        net === 'mainnet' ? '#22c55e' : '#f59e0b',
                    },
                  ]}
                />
                <Text
                  style={[
                    styles.networkText,
                    network === net && styles.networkTextActive,
                  ]}
                >
                  {net.charAt(0).toUpperCase() + net.slice(1)}
                </Text>
              </TouchableOpacity>
            ))}
          </View>
        </View>

        {/* Security */}
        <View style={styles.section}>
          <Text style={styles.sectionTitle}>Security</Text>

          <TouchableOpacity
            style={styles.settingRow}
            onPress={handleBackupSeed}
            activeOpacity={0.7}
          >
            <View style={styles.settingLeft}>
              <Text style={styles.settingLabel}>Backup Seed Phrase</Text>
              <Text style={styles.settingSubtext}>Reveal with biometric auth</Text>
            </View>
            <Text style={styles.settingArrow}>{'>'}</Text>
          </TouchableOpacity>

          <TouchableOpacity
            style={styles.settingRow}
            onPress={handleExportKeystore}
            activeOpacity={0.7}
          >
            <View style={styles.settingLeft}>
              <Text style={styles.settingLabel}>Export Keystore</Text>
              <Text style={styles.settingSubtext}>Encrypted JSON backup</Text>
            </View>
            <Text style={styles.settingArrow}>{'>'}</Text>
          </TouchableOpacity>

          <TouchableOpacity
            style={styles.settingRow}
            onPress={handleChangePin}
            activeOpacity={0.7}
          >
            <View style={styles.settingLeft}>
              <Text style={styles.settingLabel}>Change PIN</Text>
              <Text style={styles.settingSubtext}>Update your 6-digit PIN</Text>
            </View>
            <Text style={styles.settingArrow}>{'>'}</Text>
          </TouchableOpacity>
        </View>

        {/* Auto-Lock */}
        <View style={styles.section}>
          <Text style={styles.sectionTitle}>Auto-Lock Timeout</Text>
          <View style={styles.autoLockRow}>
            {AUTO_LOCK_OPTIONS.map((minutes) => (
              <TouchableOpacity
                key={minutes}
                style={[
                  styles.autoLockOption,
                  autoLockMinutes === minutes && styles.autoLockOptionActive,
                ]}
                onPress={() => handleAutoLockChange(minutes)}
                activeOpacity={0.7}
              >
                <Text
                  style={[
                    styles.autoLockText,
                    autoLockMinutes === minutes && styles.autoLockTextActive,
                  ]}
                >
                  {minutes}m
                </Text>
              </TouchableOpacity>
            ))}
          </View>
        </View>

        {/* Notifications */}
        <View style={styles.section}>
          <Text style={styles.sectionTitle}>Push Notifications</Text>

          <View style={styles.toggleRow}>
            <View style={styles.settingLeft}>
              <Text style={styles.settingLabel}>Enable Notifications</Text>
            </View>
            <Switch
              value={pushEnabled}
              onValueChange={setPushEnabled}
              trackColor={{ false: '#e5e7eb', true: '#06b6d4' }}
              thumbColor="#ffffff"
            />
          </View>

          <View style={styles.toggleRow}>
            <View style={styles.settingLeft}>
              <Text style={styles.settingLabel}>Decay Alerts</Text>
              <Text style={styles.settingSubtext}>When objects drop below 20% energy</Text>
            </View>
            <Switch
              value={decayAlerts}
              onValueChange={setDecayAlerts}
              trackColor={{ false: '#e5e7eb', true: '#06b6d4' }}
              thumbColor="#ffffff"
              disabled={!pushEnabled}
            />
          </View>

          <View style={styles.toggleRow}>
            <View style={styles.settingLeft}>
              <Text style={styles.settingLabel}>Transfer Alerts</Text>
              <Text style={styles.settingSubtext}>Incoming EVAP notifications</Text>
            </View>
            <Switch
              value={transferAlerts}
              onValueChange={setTransferAlerts}
              trackColor={{ false: '#e5e7eb', true: '#06b6d4' }}
              thumbColor="#ffffff"
              disabled={!pushEnabled}
            />
          </View>
        </View>

        {/* Advanced */}
        <View style={styles.section}>
          <Text style={styles.sectionTitle}>Advanced</Text>

          <TouchableOpacity
            style={styles.settingRow}
            onPress={() => navigation.navigate('HardwareWallet')}
            activeOpacity={0.7}
          >
            <View style={styles.settingLeft}>
              <Text style={styles.settingLabel}>Hardware Wallet</Text>
              <Text style={styles.settingSubtext}>Connect Ledger via Bluetooth</Text>
            </View>
            <Text style={styles.settingArrow}>{'>'}</Text>
          </TouchableOpacity>

          <TouchableOpacity
            style={styles.settingRow}
            onPress={() => navigation.navigate('WalletConnect')}
            activeOpacity={0.7}
          >
            <View style={styles.settingLeft}>
              <Text style={styles.settingLabel}>WalletConnect</Text>
              <Text style={styles.settingSubtext}>Connect to dApps via QR code</Text>
            </View>
            <Text style={styles.settingArrow}>{'>'}</Text>
          </TouchableOpacity>

          <TouchableOpacity
            style={styles.settingRow}
            onPress={() => navigation.navigate('OfflineMode')}
            activeOpacity={0.7}
          >
            <View style={styles.settingLeft}>
              <Text style={styles.settingLabel}>Offline Queue</Text>
              <Text style={styles.settingSubtext}>Manage queued transactions</Text>
            </View>
            <Text style={styles.settingArrow}>{'>'}</Text>
          </TouchableOpacity>
        </View>

        {/* Danger Zone */}
        <View style={styles.section}>
          <Text style={[styles.sectionTitle, { color: '#ef4444' }]}>Danger Zone</Text>
          <TouchableOpacity
            style={styles.deleteRow}
            onPress={handleDeleteWallet}
            activeOpacity={0.7}
          >
            <View style={styles.settingLeft}>
              <Text style={styles.deleteLabel}>Delete Wallet</Text>
              <Text style={styles.settingSubtext}>Remove all data from this device</Text>
            </View>
            <Text style={styles.settingArrow}>{'>'}</Text>
          </TouchableOpacity>
        </View>

        {/* App Info */}
        <View style={styles.footer}>
          <Text style={styles.footerText}>EvaporChain Mobile Wallet v0.1.0</Text>
          <Text style={styles.footerSubtext}>
            Connected to {network}
          </Text>
        </View>
      </ScrollView>

      {/* PIN Input Modal */}
      <Modal visible={pinModalMode !== null} transparent animationType="fade">
        <View style={styles.modalOverlay}>
          <View style={styles.modalCard}>
            <Text style={styles.modalTitle}>{getPinModalTitle()}</Text>

            <TextInput
              style={styles.pinInput}
              value={pinInput}
              onChangeText={(text) => {
                setPinError('');
                setPinInput(text.replace(/[^0-9]/g, '').slice(0, 6));
              }}
              keyboardType="number-pad"
              secureTextEntry
              maxLength={6}
              placeholder="6-digit PIN"
              placeholderTextColor="#9ca3af"
              autoFocus
            />

            {pinError ? <Text style={styles.pinErrorText}>{pinError}</Text> : null}

            <View style={styles.modalActions}>
              <TouchableOpacity
                style={styles.modalCancel}
                onPress={closePinModal}
                activeOpacity={0.7}
              >
                <Text style={styles.modalCancelText}>Cancel</Text>
              </TouchableOpacity>
              <TouchableOpacity
                style={[styles.modalConfirm, pinInput.length < 6 && styles.modalConfirmDisabled]}
                onPress={handlePinSubmit}
                disabled={pinInput.length < 6}
                activeOpacity={0.7}
              >
                <Text style={styles.modalConfirmText}>Confirm</Text>
              </TouchableOpacity>
            </View>
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
  scroll: {
    paddingBottom: 40,
  },
  section: {
    marginTop: 20,
    marginHorizontal: 16,
  },
  sectionTitle: {
    fontSize: 13,
    fontWeight: '600',
    color: '#6b7280',
    textTransform: 'uppercase',
    letterSpacing: 0.5,
    marginBottom: 10,
  },
  networkRow: {
    flexDirection: 'row',
    gap: 10,
  },
  networkOption: {
    flex: 1,
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'center',
    backgroundColor: '#ffffff',
    borderRadius: 12,
    paddingVertical: 14,
    borderWidth: 2,
    borderColor: '#e5e7eb',
    gap: 8,
    minHeight: 48,
  },
  networkOptionActive: {
    borderColor: '#06b6d4',
    backgroundColor: '#ecfeff',
  },
  networkDot: {
    width: 8,
    height: 8,
    borderRadius: 4,
  },
  networkText: {
    fontSize: 15,
    fontWeight: '600',
    color: '#6b7280',
  },
  networkTextActive: {
    color: '#06b6d4',
  },
  settingRow: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
    backgroundColor: '#ffffff',
    borderRadius: 12,
    padding: 16,
    marginBottom: 8,
    minHeight: 56,
  },
  settingLeft: {
    flex: 1,
  },
  settingLabel: {
    fontSize: 15,
    fontWeight: '500',
    color: '#111827',
  },
  settingSubtext: {
    fontSize: 12,
    color: '#9ca3af',
    marginTop: 2,
  },
  settingArrow: {
    fontSize: 16,
    color: '#9ca3af',
    fontWeight: '600',
    marginLeft: 8,
  },
  autoLockRow: {
    flexDirection: 'row',
    gap: 8,
  },
  autoLockOption: {
    flex: 1,
    backgroundColor: '#ffffff',
    borderRadius: 10,
    paddingVertical: 12,
    alignItems: 'center',
    borderWidth: 2,
    borderColor: '#e5e7eb',
    minHeight: 44,
    justifyContent: 'center',
  },
  autoLockOptionActive: {
    borderColor: '#8b5cf6',
    backgroundColor: '#f5f3ff',
  },
  autoLockText: {
    fontSize: 14,
    fontWeight: '600',
    color: '#6b7280',
  },
  autoLockTextActive: {
    color: '#8b5cf6',
  },
  toggleRow: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
    backgroundColor: '#ffffff',
    borderRadius: 12,
    padding: 16,
    marginBottom: 8,
    minHeight: 56,
  },
  footer: {
    alignItems: 'center',
    marginTop: 32,
    paddingVertical: 16,
  },
  footerText: {
    fontSize: 13,
    color: '#9ca3af',
    fontWeight: '500',
  },
  footerSubtext: {
    fontSize: 12,
    color: '#d1d5db',
    marginTop: 4,
  },
  deleteRow: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
    backgroundColor: '#fef2f2',
    borderWidth: 1,
    borderColor: '#fecaca',
    borderRadius: 12,
    padding: 16,
    marginBottom: 8,
    minHeight: 56,
  },
  deleteLabel: {
    fontSize: 15,
    fontWeight: '500',
    color: '#ef4444',
  },
  // PIN Modal
  modalOverlay: {
    flex: 1,
    backgroundColor: 'rgba(0,0,0,0.5)',
    justifyContent: 'center',
    alignItems: 'center',
    paddingHorizontal: 32,
  },
  modalCard: {
    backgroundColor: '#ffffff',
    borderRadius: 16,
    padding: 24,
    width: '100%',
    maxWidth: 340,
  },
  modalTitle: {
    fontSize: 17,
    fontWeight: '600',
    color: '#111827',
    textAlign: 'center',
    marginBottom: 20,
  },
  pinInput: {
    backgroundColor: '#f3f4f6',
    borderRadius: 12,
    paddingHorizontal: 16,
    paddingVertical: 14,
    fontSize: 24,
    fontWeight: '600',
    textAlign: 'center',
    color: '#111827',
    letterSpacing: 8,
  },
  pinErrorText: {
    color: '#ef4444',
    fontSize: 13,
    textAlign: 'center',
    marginTop: 8,
  },
  modalActions: {
    flexDirection: 'row',
    gap: 12,
    marginTop: 20,
  },
  modalCancel: {
    flex: 1,
    backgroundColor: '#f3f4f6',
    borderRadius: 12,
    paddingVertical: 14,
    alignItems: 'center',
    minHeight: 48,
    justifyContent: 'center',
  },
  modalCancelText: {
    fontSize: 15,
    fontWeight: '600',
    color: '#6b7280',
  },
  modalConfirm: {
    flex: 1,
    backgroundColor: '#06b6d4',
    borderRadius: 12,
    paddingVertical: 14,
    alignItems: 'center',
    minHeight: 48,
    justifyContent: 'center',
  },
  modalConfirmDisabled: {
    opacity: 0.5,
  },
  modalConfirmText: {
    fontSize: 15,
    fontWeight: '600',
    color: '#ffffff',
  },
});

export default SettingsScreen;
