/**
 * UnlockScreen — PIN pad and biometric unlock
 */

import React, { useState, useCallback, useEffect } from 'react';
import {
  View,
  Text,
  TouchableOpacity,
  StyleSheet,
  Alert,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import * as LocalAuthentication from 'expo-local-authentication';
import type { NativeStackNavigationProp } from '@react-navigation/native-stack';
import type { RootStackParamList } from '../navigation/AppNavigator';
import { keystore } from '../utils/keystore';

type Props = {
  navigation: NativeStackNavigationProp<RootStackParamList, 'Unlock'>;
};

const PIN_LENGTH = 6;
const PIN_KEYS = [
  ['1', '2', '3'],
  ['4', '5', '6'],
  ['7', '8', '9'],
  ['', '0', 'DEL'],
];

const UnlockScreen: React.FC<Props> = ({ navigation }) => {
  const [pin, setPin] = useState('');
  const [biometricAvailable, setBiometricAvailable] = useState(false);

  useEffect(() => {
    checkBiometrics();
  }, []);

  const checkBiometrics = async () => {
    const compatible = await LocalAuthentication.hasHardwareAsync();
    const enrolled = await LocalAuthentication.isEnrolledAsync();
    setBiometricAvailable(compatible && enrolled);
  };

  const handlePinPress = useCallback(
    (key: string) => {
      if (key === 'DEL') {
        setPin((prev) => prev.slice(0, -1));
        return;
      }
      if (key === '') return;

      const newPin = pin + key;
      if (newPin.length > PIN_LENGTH) return;

      setPin(newPin);

      if (newPin.length === PIN_LENGTH) {
        verifyPin(newPin);
      }
    },
    [pin]
  );

  const verifyPin = async (enteredPin: string) => {
    const valid = await keystore.verifyPin(enteredPin);
    if (valid) {
      navigation.replace('MainTabs');
    } else {
      Alert.alert('Incorrect PIN', 'Please try again.');
      setPin('');
    }
  };

  const handleBiometric = async () => {
    const result = await LocalAuthentication.authenticateAsync({
      promptMessage: 'Unlock EvaporChain Wallet',
      cancelLabel: 'Use PIN',
      disableDeviceFallback: false,
    });

    if (result.success) {
      navigation.replace('MainTabs');
    }
  };

  return (
    <SafeAreaView style={styles.container}>
      {/* Branding */}
      <View style={styles.branding}>
        <View style={styles.logoCircle}>
          <Text style={styles.logoText}>EC</Text>
        </View>
        <Text style={styles.title}>EvaporChain</Text>
        <Text style={styles.subtitle}>Mobile Wallet</Text>
      </View>

      {/* PIN dots */}
      <View style={styles.pinDots}>
        {Array.from({ length: PIN_LENGTH }).map((_, i) => (
          <View
            key={i}
            style={[styles.dot, i < pin.length && styles.dotFilled]}
          />
        ))}
      </View>

      {/* PIN pad */}
      <View style={styles.pinPad}>
        {PIN_KEYS.map((row, rowIdx) => (
          <View key={rowIdx} style={styles.pinRow}>
            {row.map((key) => (
              <TouchableOpacity
                key={key || 'empty'}
                style={[
                  styles.pinKey,
                  key === '' && styles.pinKeyHidden,
                ]}
                onPress={() => handlePinPress(key)}
                disabled={key === ''}
                activeOpacity={0.6}
              >
                <Text
                  style={[
                    styles.pinKeyText,
                    key === 'DEL' && styles.pinKeyDeleteText,
                  ]}
                >
                  {key === 'DEL' ? 'Delete' : key}
                </Text>
              </TouchableOpacity>
            ))}
          </View>
        ))}
      </View>

      {/* Biometric button */}
      {biometricAvailable && (
        <TouchableOpacity
          style={styles.biometricButton}
          onPress={handleBiometric}
          activeOpacity={0.7}
        >
          <Text style={styles.biometricText}>Use Face ID / Fingerprint</Text>
        </TouchableOpacity>
      )}
    </SafeAreaView>
  );
};

const styles = StyleSheet.create({
  container: {
    flex: 1,
    backgroundColor: '#ffffff',
    alignItems: 'center',
    justifyContent: 'center',
    paddingHorizontal: 24,
  },
  branding: {
    alignItems: 'center',
    marginBottom: 40,
  },
  logoCircle: {
    width: 72,
    height: 72,
    borderRadius: 36,
    backgroundColor: '#06b6d4',
    alignItems: 'center',
    justifyContent: 'center',
    marginBottom: 12,
  },
  logoText: {
    color: '#ffffff',
    fontSize: 28,
    fontWeight: '800',
    letterSpacing: 1,
  },
  title: {
    fontSize: 24,
    fontWeight: '700',
    color: '#111827',
  },
  subtitle: {
    fontSize: 14,
    color: '#6b7280',
    marginTop: 4,
  },
  pinDots: {
    flexDirection: 'row',
    gap: 16,
    marginBottom: 32,
  },
  dot: {
    width: 14,
    height: 14,
    borderRadius: 7,
    borderWidth: 2,
    borderColor: '#d1d5db',
    backgroundColor: 'transparent',
  },
  dotFilled: {
    backgroundColor: '#06b6d4',
    borderColor: '#06b6d4',
  },
  pinPad: {
    width: '100%',
    maxWidth: 300,
    gap: 12,
  },
  pinRow: {
    flexDirection: 'row',
    justifyContent: 'center',
    gap: 16,
  },
  pinKey: {
    width: 72,
    height: 72,
    borderRadius: 36,
    backgroundColor: '#f3f4f6',
    alignItems: 'center',
    justifyContent: 'center',
  },
  pinKeyHidden: {
    backgroundColor: 'transparent',
  },
  pinKeyText: {
    fontSize: 24,
    fontWeight: '600',
    color: '#111827',
  },
  pinKeyDeleteText: {
    fontSize: 14,
    color: '#6b7280',
  },
  biometricButton: {
    marginTop: 24,
    paddingVertical: 14,
    paddingHorizontal: 32,
    backgroundColor: '#8b5cf6',
    borderRadius: 12,
    minHeight: 48,
    alignItems: 'center',
    justifyContent: 'center',
  },
  biometricText: {
    color: '#ffffff',
    fontSize: 15,
    fontWeight: '600',
  },
});

export default UnlockScreen;
