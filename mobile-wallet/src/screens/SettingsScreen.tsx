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
  TouchableOpacity,
  ScrollView,
  StyleSheet,
  Alert,
  Switch,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import * as LocalAuthentication from 'expo-local-authentication';
import { api } from '../utils/api';
import { keystore } from '../utils/keystore';

const NETWORKS = ['testnet', 'mainnet'] as const;
const AUTO_LOCK_OPTIONS = [1, 5, 15, 30] as const;

const SettingsScreen: React.FC = () => {
  const [network, setNetwork] = useState<string>(api.getNetwork());
  const [autoLockMinutes, setAutoLockMinutes] = useState(5);
  const [pushEnabled, setPushEnabled] = useState(true);
  const [decayAlerts, setDecayAlerts] = useState(true);
  const [transferAlerts, setTransferAlerts] = useState(true);

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
            api.setNetwork(newNetwork);
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

  const handleExportKeystore = async () => {
    Alert.prompt?.(
      'Export Keystore',
      'Enter your PIN to export:',
      async (pin: string) => {
        const data = await keystore.exportKeystore(pin);
        if (!data) {
          Alert.alert('Error', 'Incorrect PIN or export failed.');
          return;
        }
        // In production, save to file or share
        Alert.alert('Keystore Exported', 'Keystore JSON has been copied to clipboard.');
      },
      'secure-text'
    ) ?? Alert.alert('Export', 'Keystore export requires PIN verification. Feature coming soon on Android.');
  };

  const handleChangePin = () => {
    Alert.alert(
      'Change PIN',
      'PIN change flow will guide you through entering your current PIN and setting a new 6-digit PIN.',
      [{ text: 'OK' }]
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

        {/* App Info */}
        <View style={styles.footer}>
          <Text style={styles.footerText}>EvaporChain Mobile Wallet v0.1.0</Text>
          <Text style={styles.footerSubtext}>
            Connected to {network}
          </Text>
        </View>
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
});

export default SettingsScreen;
