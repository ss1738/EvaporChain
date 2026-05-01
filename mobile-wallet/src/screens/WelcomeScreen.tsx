/**
 * WelcomeScreen — First-time onboarding
 *
 * Shown when no wallet exists on device. Offers create or import.
 */

import React from 'react';
import {
  View,
  Text,
  TouchableOpacity,
  StyleSheet,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import type { NativeStackNavigationProp } from '@react-navigation/native-stack';
import type { RootStackParamList } from '../navigation/AppNavigator';

type Props = {
  navigation: NativeStackNavigationProp<RootStackParamList, 'Welcome'>;
};

const WelcomeScreen: React.FC<Props> = ({ navigation }) => {
  return (
    <SafeAreaView style={styles.container}>
      <View style={styles.content}>
        {/* Logo */}
        <View style={styles.logoCircle}>
          <Text style={styles.logoText}>EC</Text>
        </View>
        <Text style={styles.title}>EvaporChain</Text>
        <Text style={styles.subtitle}>Post-Quantum Mobile Wallet</Text>

        {/* Features */}
        <View style={styles.features}>
          <View style={styles.feature}>
            <View style={[styles.featureDot, { backgroundColor: '#06b6d4' }]} />
            <Text style={styles.featureText}>ML-DSA quantum-safe signatures</Text>
          </View>
          <View style={styles.feature}>
            <View style={[styles.featureDot, { backgroundColor: '#22c55e' }]} />
            <Text style={styles.featureText}>Energy-based state decay</Text>
          </View>
          <View style={styles.feature}>
            <View style={[styles.featureDot, { backgroundColor: '#8b5cf6' }]} />
            <Text style={styles.featureText}>Biometric security</Text>
          </View>
        </View>
      </View>

      {/* Actions */}
      <View style={styles.actions}>
        <TouchableOpacity
          testID="welcome-create-wallet"
          style={styles.createButton}
          onPress={() => navigation.navigate('CreateWallet')}
          activeOpacity={0.7}
        >
          <Text style={styles.createButtonText}>Create New Wallet</Text>
        </TouchableOpacity>

        <TouchableOpacity
          testID="welcome-import-wallet"
          style={styles.importButton}
          onPress={() => navigation.navigate('ImportWallet')}
          activeOpacity={0.7}
        >
          <Text style={styles.importButtonText}>Import from Seed Phrase</Text>
        </TouchableOpacity>
      </View>
    </SafeAreaView>
  );
};

const styles = StyleSheet.create({
  container: {
    flex: 1,
    backgroundColor: '#ffffff',
    paddingHorizontal: 24,
  },
  content: {
    flex: 1,
    alignItems: 'center',
    justifyContent: 'center',
  },
  logoCircle: {
    width: 96,
    height: 96,
    borderRadius: 48,
    backgroundColor: '#06b6d4',
    alignItems: 'center',
    justifyContent: 'center',
    marginBottom: 16,
  },
  logoText: {
    color: '#ffffff',
    fontSize: 36,
    fontWeight: '800',
    letterSpacing: 1,
  },
  title: {
    fontSize: 32,
    fontWeight: '700',
    color: '#111827',
  },
  subtitle: {
    fontSize: 15,
    color: '#6b7280',
    marginTop: 4,
  },
  features: {
    marginTop: 40,
    gap: 14,
  },
  feature: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 12,
  },
  featureDot: {
    width: 8,
    height: 8,
    borderRadius: 4,
  },
  featureText: {
    fontSize: 15,
    color: '#374151',
  },
  actions: {
    paddingBottom: 32,
    gap: 12,
  },
  createButton: {
    backgroundColor: '#06b6d4',
    borderRadius: 14,
    paddingVertical: 16,
    alignItems: 'center',
    minHeight: 52,
    justifyContent: 'center',
  },
  createButtonText: {
    color: '#ffffff',
    fontSize: 17,
    fontWeight: '700',
  },
  importButton: {
    backgroundColor: '#ffffff',
    borderWidth: 2,
    borderColor: '#06b6d4',
    borderRadius: 14,
    paddingVertical: 16,
    alignItems: 'center',
    minHeight: 52,
    justifyContent: 'center',
  },
  importButtonText: {
    color: '#06b6d4',
    fontSize: 17,
    fontWeight: '700',
  },
});

export default WelcomeScreen;
