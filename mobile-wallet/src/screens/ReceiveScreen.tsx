/**
 * ReceiveScreen — Display QR code and address for receiving EVAP
 */

import React, { useState, useEffect } from 'react';
import {
  View,
  Text,
  TouchableOpacity,
  StyleSheet,
  Alert,
  Share,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import * as Clipboard from 'expo-clipboard';
import { keystore } from '../utils/keystore';

const ReceiveScreen: React.FC = () => {
  const [address, setAddress] = useState<string>('');
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    loadAddress();
  }, []);

  const loadAddress = async () => {
    const addr = await keystore.getAddress();
    if (addr) setAddress(addr);
  };

  const handleCopy = async () => {
    if (!address) return;
    await Clipboard.setStringAsync(address);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  const handleShare = async () => {
    if (!address) return;
    try {
      await Share.share({
        message: `My EvaporChain wallet address: ${address}`,
      });
    } catch {
      Alert.alert('Error', 'Could not open share dialog.');
    }
  };

  return (
    <SafeAreaView style={styles.container} edges={['bottom']}>
      <View style={styles.content}>
        {/* QR Code placeholder */}
        <View style={styles.qrContainer}>
          <View style={styles.qrPlaceholder}>
            {/* In production, use react-native-qrcode-svg here */}
            <Text style={styles.qrText}>QR</Text>
            <Text style={styles.qrSubtext}>
              {address ? address.slice(0, 12) + '...' : 'Loading...'}
            </Text>
          </View>
        </View>

        <Text style={styles.instructions}>
          Scan this QR code or share your address to receive EVAP tokens.
        </Text>

        {/* Address display */}
        <View style={styles.addressBox}>
          <Text style={styles.addressLabel}>Your Address</Text>
          <Text style={styles.addressText} selectable>
            {address || 'Loading...'}
          </Text>
        </View>

        {/* Actions */}
        <View style={styles.actions}>
          <TouchableOpacity
            style={styles.copyButton}
            onPress={handleCopy}
            activeOpacity={0.7}
          >
            <Text style={styles.copyButtonText}>
              {copied ? 'Copied!' : 'Copy Address'}
            </Text>
          </TouchableOpacity>

          <TouchableOpacity
            style={styles.shareButton}
            onPress={handleShare}
            activeOpacity={0.7}
          >
            <Text style={styles.shareButtonText}>Share</Text>
          </TouchableOpacity>
        </View>
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
    alignItems: 'center',
    paddingHorizontal: 24,
    paddingTop: 32,
  },
  qrContainer: {
    backgroundColor: '#ffffff',
    borderRadius: 20,
    padding: 24,
    shadowColor: '#000',
    shadowOffset: { width: 0, height: 2 },
    shadowOpacity: 0.05,
    shadowRadius: 8,
    elevation: 2,
    marginBottom: 24,
  },
  qrPlaceholder: {
    width: 220,
    height: 220,
    backgroundColor: '#f3f4f6',
    borderRadius: 12,
    alignItems: 'center',
    justifyContent: 'center',
    borderWidth: 2,
    borderColor: '#e5e7eb',
    borderStyle: 'dashed',
  },
  qrText: {
    fontSize: 48,
    fontWeight: '700',
    color: '#06b6d4',
  },
  qrSubtext: {
    fontSize: 12,
    color: '#9ca3af',
    marginTop: 8,
  },
  instructions: {
    fontSize: 14,
    color: '#6b7280',
    textAlign: 'center',
    marginBottom: 24,
    lineHeight: 20,
  },
  addressBox: {
    backgroundColor: '#ffffff',
    borderRadius: 12,
    padding: 16,
    width: '100%',
    marginBottom: 24,
  },
  addressLabel: {
    fontSize: 12,
    color: '#9ca3af',
    fontWeight: '500',
    marginBottom: 8,
  },
  addressText: {
    fontSize: 14,
    color: '#111827',
    fontWeight: '500',
    fontFamily: 'monospace',
    lineHeight: 20,
  },
  actions: {
    flexDirection: 'row',
    gap: 12,
    width: '100%',
  },
  copyButton: {
    flex: 1,
    backgroundColor: '#06b6d4',
    borderRadius: 12,
    paddingVertical: 14,
    alignItems: 'center',
    minHeight: 48,
    justifyContent: 'center',
  },
  copyButtonText: {
    color: '#ffffff',
    fontSize: 15,
    fontWeight: '600',
  },
  shareButton: {
    flex: 1,
    backgroundColor: '#8b5cf6',
    borderRadius: 12,
    paddingVertical: 14,
    alignItems: 'center',
    minHeight: 48,
    justifyContent: 'center',
  },
  shareButtonText: {
    color: '#ffffff',
    fontSize: 15,
    fontWeight: '600',
  },
});

export default ReceiveScreen;
