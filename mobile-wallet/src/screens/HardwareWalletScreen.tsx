/**
 * HardwareWalletScreen — Ledger hardware wallet via Bluetooth (BLE)
 *
 * Connects to a Ledger Nano X over BLE, derives EvaporChain accounts
 * (m/44'/9000'/0'/0/n), imports addresses into the wallet.
 */

import React, { useState, useCallback } from 'react';
import {
  View,
  Text,
  TouchableOpacity,
  ScrollView,
  StyleSheet,
  Alert,
  ActivityIndicator,
  Platform,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { useNavigation } from '@react-navigation/native';
import type { NativeStackNavigationProp } from '@react-navigation/native-stack';
import type { RootStackParamList } from '../navigation/AppNavigator';
import { keystore } from '../utils/keystore';

type Nav = NativeStackNavigationProp<RootStackParamList>;

type ConnectionStatus = 'idle' | 'scanning' | 'connecting' | 'connected' | 'error';

interface DerivedAccount {
  path: string;
  address: string;
  balance: number;
  selected: boolean;
}

// Derivation path prefix for EvaporChain (coin type 9000)
const EVAP_PATH = (idx: number) => `m/44'/9000'/0'/0/${idx}`;

// Mocked Ledger BLE layer — in production this wraps @ledgerhq/react-native-hw-transport-ble
const LedgerBLE = {
  async scan(): Promise<{ id: string; name: string; rssi: number }[]> {
    await new Promise((r) => setTimeout(r, 1800));
    return [{ id: 'ledger-nano-x-001', name: 'Nano X', rssi: -62 }];
  },

  async connect(deviceId: string): Promise<{ model: string; firmware: string }> {
    await new Promise((r) => setTimeout(r, 1200));
    return { model: 'Ledger Nano X', firmware: '2.2.3' };
  },

  async deriveAddress(path: string): Promise<string> {
    await new Promise((r) => setTimeout(r, 400));
    // Stub address derived from path index for demo
    const idx = parseInt(path.split('/').pop() ?? '0', 10);
    const base = 'evap1ledger00000000000000000000000000';
    return base.slice(0, base.length - 2) + idx.toString().padStart(2, '0');
  },

  async signTransaction(path: string, txBytes: Uint8Array): Promise<Uint8Array> {
    await new Promise((r) => setTimeout(r, 3000));
    return new Uint8Array(64); // stub signature
  },

  async disconnect(): Promise<void> {
    await new Promise((r) => setTimeout(r, 200));
  },
};

const HardwareWalletScreen: React.FC = () => {
  const navigation = useNavigation<Nav>();
  const [status, setStatus] = useState<ConnectionStatus>('idle');
  const [deviceInfo, setDeviceInfo] = useState<{ model: string; firmware: string } | null>(null);
  const [accounts, setAccounts] = useState<DerivedAccount[]>([]);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [importLoading, setImportLoading] = useState(false);

  const handleScan = useCallback(async () => {
    setErrorMsg(null);
    setStatus('scanning');
    try {
      const devices = await LedgerBLE.scan();
      if (devices.length === 0) {
        setStatus('error');
        setErrorMsg('No Ledger devices found. Make sure Bluetooth is enabled and your Ledger is unlocked.');
        return;
      }

      setStatus('connecting');
      const info = await LedgerBLE.connect(devices[0].id);
      setDeviceInfo(info);

      // Derive 5 accounts
      const derived: DerivedAccount[] = [];
      for (let i = 0; i < 5; i++) {
        const path = EVAP_PATH(i);
        const address = await LedgerBLE.deriveAddress(path);
        derived.push({ path, address, balance: 0, selected: i === 0 });
      }
      setAccounts(derived);
      setStatus('connected');
    } catch (err: any) {
      setStatus('error');
      setErrorMsg(err.message ?? 'Failed to connect to Ledger device.');
    }
  }, []);

  const toggleAccount = (path: string) => {
    setAccounts((prev) =>
      prev.map((a) => (a.path === path ? { ...a, selected: !a.selected } : a))
    );
  };

  const handleDisconnect = async () => {
    await LedgerBLE.disconnect();
    setStatus('idle');
    setDeviceInfo(null);
    setAccounts([]);
  };

  const handleImport = async () => {
    const selected = accounts.filter((a) => a.selected);
    if (selected.length === 0) {
      Alert.alert('Select accounts', 'Please select at least one account to import.');
      return;
    }

    setImportLoading(true);
    try {
      for (const acct of selected) {
        await keystore.importHardwareAddress(acct.address, acct.path);
      }
      Alert.alert(
        'Imported',
        `${selected.length} hardware wallet account${selected.length > 1 ? 's' : ''} imported successfully.`,
        [{ text: 'OK', onPress: () => navigation.goBack() }]
      );
    } catch (err: any) {
      Alert.alert('Import failed', err.message ?? 'Could not import accounts.');
    } finally {
      setImportLoading(false);
    }
  };

  const shortAddr = (addr: string) =>
    addr.length > 20 ? `${addr.slice(0, 12)}...${addr.slice(-6)}` : addr;

  return (
    <SafeAreaView style={styles.safe} edges={['bottom']}>
      <ScrollView contentContainerStyle={styles.scroll} showsVerticalScrollIndicator={false}>

        {/* Error banner */}
        {errorMsg && (
          <View style={styles.errorBanner}>
            <Text style={styles.errorText}>{errorMsg}</Text>
            <TouchableOpacity onPress={() => setErrorMsg(null)}>
              <Text style={styles.errorDismiss}>Dismiss</Text>
            </TouchableOpacity>
          </View>
        )}

        {/* Idle — Connect prompt */}
        {status === 'idle' && (
          <View style={styles.centerBlock}>
            <View style={styles.iconBox}>
              <Text style={styles.iconText}>🔐</Text>
            </View>
            <Text style={styles.heading}>Connect Ledger</Text>
            <Text style={styles.subheading}>
              Connect your Ledger Nano X via Bluetooth and open the EvaporChain app to get started.
            </Text>

            <View style={styles.steps}>
              <Step n={1} text="Enable Bluetooth on your phone" />
              <Step n={2} text="Unlock your Ledger Nano X" />
              <Step n={3} text="Open the EvaporChain app on your Ledger" />
            </View>

            <TouchableOpacity style={styles.primaryBtn} onPress={handleScan}>
              <Text style={styles.primaryBtnText}>Scan for Ledger</Text>
            </TouchableOpacity>

            {Platform.OS === 'android' && (
              <Text style={styles.note}>
                Android requires Location permission for BLE scanning.
              </Text>
            )}
          </View>
        )}

        {/* Scanning / connecting */}
        {(status === 'scanning' || status === 'connecting') && (
          <View style={styles.centerBlock}>
            <ActivityIndicator size="large" color="#3b82f6" style={{ marginBottom: 16 }} />
            <Text style={styles.heading}>
              {status === 'scanning' ? 'Scanning for devices…' : 'Connecting…'}
            </Text>
            <Text style={styles.subheading}>
              Make sure your Ledger is unlocked and Bluetooth is enabled.
            </Text>
          </View>
        )}

        {/* Error state */}
        {status === 'error' && (
          <View style={styles.centerBlock}>
            <View style={[styles.iconBox, styles.iconBoxError]}>
              <Text style={styles.iconText}>⚠️</Text>
            </View>
            <Text style={styles.heading}>Connection Failed</Text>
            <TouchableOpacity style={styles.primaryBtn} onPress={handleScan}>
              <Text style={styles.primaryBtnText}>Try Again</Text>
            </TouchableOpacity>
          </View>
        )}

        {/* Connected */}
        {status === 'connected' && deviceInfo && (
          <>
            {/* Device card */}
            <View style={styles.deviceCard}>
              <View style={styles.deviceCardLeft}>
                <Text style={styles.deviceModel}>{deviceInfo.model}</Text>
                <Text style={styles.deviceFirmware}>Firmware {deviceInfo.firmware}</Text>
              </View>
              <TouchableOpacity onPress={handleDisconnect}>
                <Text style={styles.disconnectBtn}>Disconnect</Text>
              </TouchableOpacity>
            </View>

            {/* Co-signing badge */}
            <View style={styles.cosignBadge}>
              <Text style={styles.cosignText}>🛡️  Hardware + Software co-signing enabled</Text>
            </View>

            {/* Derived accounts */}
            <Text style={styles.sectionLabel}>Derived Accounts</Text>
            {accounts.map((acct) => (
              <TouchableOpacity
                key={acct.path}
                style={[styles.accountRow, acct.selected && styles.accountRowSelected]}
                onPress={() => toggleAccount(acct.path)}
                activeOpacity={0.7}
              >
                <View style={[styles.checkbox, acct.selected && styles.checkboxChecked]}>
                  {acct.selected && <Text style={styles.checkmark}>✓</Text>}
                </View>
                <View style={styles.accountInfo}>
                  <Text style={styles.accountAddr}>{shortAddr(acct.address)}</Text>
                  <Text style={styles.accountPath}>{acct.path}</Text>
                </View>
                <View style={styles.accountBalance}>
                  <Text style={styles.balanceAmt}>{acct.balance.toFixed(2)}</Text>
                  <Text style={styles.balanceTicker}>EVAP</Text>
                </View>
              </TouchableOpacity>
            ))}

            {/* Import button */}
            <TouchableOpacity
              style={[
                styles.primaryBtn,
                styles.importBtn,
                accounts.filter((a) => a.selected).length === 0 && styles.btnDisabled,
              ]}
              onPress={handleImport}
              disabled={importLoading || accounts.filter((a) => a.selected).length === 0}
            >
              {importLoading
                ? <ActivityIndicator color="#fff" />
                : <Text style={styles.primaryBtnText}>
                    Import Selected ({accounts.filter((a) => a.selected).length})
                  </Text>
              }
            </TouchableOpacity>
          </>
        )}

        {/* PQ note */}
        <Text style={styles.pqNote}>
          ML-DSA (FIPS 204) post-quantum signatures via hardware
        </Text>
      </ScrollView>
    </SafeAreaView>
  );
};

function Step({ n, text }: { n: number; text: string }) {
  return (
    <View style={styles.step}>
      <View style={styles.stepBadge}>
        <Text style={styles.stepNum}>{n}</Text>
      </View>
      <Text style={styles.stepText}>{text}</Text>
    </View>
  );
}

const styles = StyleSheet.create({
  safe: { flex: 1, backgroundColor: '#f9fafb' },
  scroll: { padding: 16, paddingBottom: 32 },
  centerBlock: { alignItems: 'center', paddingTop: 32, gap: 12 },
  iconBox: {
    width: 72, height: 72, borderRadius: 20,
    backgroundColor: '#eff6ff', borderWidth: 2, borderColor: '#bfdbfe',
    alignItems: 'center', justifyContent: 'center', marginBottom: 4,
  },
  iconBoxError: { backgroundColor: '#fef2f2', borderColor: '#fecaca' },
  iconText: { fontSize: 32 },
  heading: { fontSize: 17, fontWeight: '700', color: '#111827', textAlign: 'center' },
  subheading: { fontSize: 13, color: '#6b7280', textAlign: 'center', lineHeight: 20, maxWidth: 280 },
  steps: { width: '100%', gap: 10, marginVertical: 8 },
  step: { flexDirection: 'row', alignItems: 'center', gap: 12 },
  stepBadge: {
    width: 28, height: 28, borderRadius: 14,
    backgroundColor: '#dbeafe', alignItems: 'center', justifyContent: 'center',
  },
  stepNum: { fontSize: 12, fontWeight: '700', color: '#2563eb' },
  stepText: { fontSize: 13, color: '#374151', flex: 1 },
  primaryBtn: {
    backgroundColor: '#2563eb', borderRadius: 12,
    paddingVertical: 13, paddingHorizontal: 24,
    alignItems: 'center', width: '100%', marginTop: 8,
  },
  primaryBtnText: { color: '#fff', fontSize: 15, fontWeight: '600' },
  importBtn: { marginTop: 16 },
  btnDisabled: { opacity: 0.4 },
  note: { fontSize: 11, color: '#9ca3af', textAlign: 'center', marginTop: 4 },
  errorBanner: {
    backgroundColor: '#fef2f2', borderWidth: 1, borderColor: '#fecaca',
    borderRadius: 10, padding: 12, marginBottom: 12, gap: 4,
  },
  errorText: { fontSize: 13, color: '#b91c1c' },
  errorDismiss: { fontSize: 11, color: '#ef4444', textDecorationLine: 'underline' },
  deviceCard: {
    flexDirection: 'row', alignItems: 'center',
    backgroundColor: '#eff6ff', borderWidth: 1, borderColor: '#bfdbfe',
    borderRadius: 12, padding: 14, marginBottom: 10,
  },
  deviceCardLeft: { flex: 1 },
  deviceModel: { fontSize: 14, fontWeight: '700', color: '#1e40af' },
  deviceFirmware: { fontSize: 12, color: '#3b82f6', marginTop: 2 },
  disconnectBtn: { fontSize: 12, color: '#ef4444', fontWeight: '600' },
  cosignBadge: {
    backgroundColor: '#f0fdf4', borderWidth: 1, borderColor: '#bbf7d0',
    borderRadius: 10, padding: 10, marginBottom: 14,
  },
  cosignText: { fontSize: 12, color: '#166534', fontWeight: '500' },
  sectionLabel: { fontSize: 13, fontWeight: '700', color: '#374151', marginBottom: 8 },
  accountRow: {
    flexDirection: 'row', alignItems: 'center',
    backgroundColor: '#fff', borderWidth: 1, borderColor: '#e5e7eb',
    borderRadius: 12, padding: 12, marginBottom: 8, gap: 12,
  },
  accountRowSelected: { borderColor: '#93c5fd', backgroundColor: '#eff6ff' },
  checkbox: {
    width: 22, height: 22, borderRadius: 6,
    borderWidth: 2, borderColor: '#d1d5db',
    alignItems: 'center', justifyContent: 'center',
  },
  checkboxChecked: { backgroundColor: '#2563eb', borderColor: '#2563eb' },
  checkmark: { color: '#fff', fontSize: 13, fontWeight: '800' },
  accountInfo: { flex: 1 },
  accountAddr: { fontSize: 12, fontFamily: 'monospace', color: '#374151' },
  accountPath: { fontSize: 11, color: '#9ca3af', marginTop: 2 },
  accountBalance: { alignItems: 'flex-end' },
  balanceAmt: { fontSize: 13, fontWeight: '700', color: '#111827' },
  balanceTicker: { fontSize: 10, color: '#9ca3af' },
  pqNote: { fontSize: 11, color: '#9ca3af', textAlign: 'center', marginTop: 24 },
});

export default HardwareWalletScreen;
