/**
 * AppNavigator — React Navigation stack configuration
 *
 * Stack: Welcome/Unlock -> Home -> Send / Receive / Objects / NFTs / Swap / Settings
 *
 * On launch, checks if a wallet exists:
 *   - No wallet -> WelcomeScreen (create or import)
 *   - Wallet exists -> UnlockScreen (PIN/biometric)
 */

import React, { useState, useEffect } from 'react';
import { ActivityIndicator, View } from 'react-native';
import { NavigationContainer } from '@react-navigation/native';
import { createNativeStackNavigator } from '@react-navigation/native-stack';
import { keystore } from '../utils/keystore';

import WelcomeScreen from '../screens/WelcomeScreen';
import CreateWalletScreen from '../screens/CreateWalletScreen';
import ImportWalletScreen from '../screens/ImportWalletScreen';
import UnlockScreen from '../screens/UnlockScreen';
import HomeScreen from '../screens/HomeScreen';
import SendScreen from '../screens/SendScreen';
import ReceiveScreen from '../screens/ReceiveScreen';
import ObjectsScreen from '../screens/ObjectsScreen';
import NftScreen from '../screens/NftScreen';
import SwapScreen from '../screens/SwapScreen';
import SettingsScreen from '../screens/SettingsScreen';
import HistoryScreen from '../screens/HistoryScreen';

export type RootStackParamList = {
  Welcome: undefined;
  CreateWallet: undefined;
  ImportWallet: undefined;
  Unlock: undefined;
  Home: undefined;
  Send: { prefillAddress?: string } | undefined;
  Receive: undefined;
  Objects: undefined;
  NFTs: undefined;
  Swap: undefined;
  Settings: undefined;
  History: undefined;
};

const Stack = createNativeStackNavigator<RootStackParamList>();

const headerStyle = {
  backgroundColor: '#ffffff',
};

const headerTintColor = '#111827';

export const AppNavigator: React.FC = () => {
  const [loading, setLoading] = useState(true);
  const [hasWallet, setHasWallet] = useState(false);

  useEffect(() => {
    keystore.hasWallet().then((exists) => {
      setHasWallet(exists);
      setLoading(false);
    });
  }, []);

  if (loading) {
    return (
      <View style={{ flex: 1, alignItems: 'center', justifyContent: 'center', backgroundColor: '#ffffff' }}>
        <ActivityIndicator size="large" color="#06b6d4" />
      </View>
    );
  }

  return (
    <NavigationContainer>
      <Stack.Navigator
        initialRouteName={hasWallet ? 'Unlock' : 'Welcome'}
        screenOptions={{
          headerStyle,
          headerTintColor,
          headerTitleStyle: { fontWeight: '600' as const },
          headerShadowVisible: false,
          headerBackTitleVisible: false,
          contentStyle: { backgroundColor: '#f9fafb' },
          animation: 'slide_from_right',
        }}
      >
        {/* Onboarding */}
        <Stack.Screen
          name="Welcome"
          component={WelcomeScreen}
          options={{ headerShown: false }}
        />
        <Stack.Screen
          name="CreateWallet"
          component={CreateWalletScreen}
          options={{ title: 'Create Wallet' }}
        />
        <Stack.Screen
          name="ImportWallet"
          component={ImportWalletScreen}
          options={{ title: 'Import Wallet' }}
        />

        {/* Auth */}
        <Stack.Screen
          name="Unlock"
          component={UnlockScreen}
          options={{ headerShown: false }}
        />

        {/* Main */}
        <Stack.Screen
          name="Home"
          component={HomeScreen}
          options={{
            title: 'EvaporChain',
            headerLeft: () => null,
            gestureEnabled: false,
          }}
        />
        <Stack.Screen
          name="Send"
          component={SendScreen}
          options={{ title: 'Send EVAP' }}
        />
        <Stack.Screen
          name="Receive"
          component={ReceiveScreen}
          options={{ title: 'Receive EVAP' }}
        />
        <Stack.Screen
          name="Objects"
          component={ObjectsScreen}
          options={{ title: 'My Objects' }}
        />
        <Stack.Screen
          name="NFTs"
          component={NftScreen}
          options={{ title: 'My NFTs' }}
        />
        <Stack.Screen
          name="Swap"
          component={SwapScreen}
          options={{ title: 'Swap' }}
        />
        <Stack.Screen
          name="History"
          component={HistoryScreen}
          options={{ title: 'Transaction History' }}
        />
        <Stack.Screen
          name="Settings"
          component={SettingsScreen}
          options={{ title: 'Settings' }}
        />
      </Stack.Navigator>
    </NavigationContainer>
  );
};

export default AppNavigator;
