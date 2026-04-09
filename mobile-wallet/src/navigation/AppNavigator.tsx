/**
 * AppNavigator — Navigation structure
 *
 * Root Stack: Welcome / Unlock / MainTabs / detail screens
 * MainTabs: Home | Energy | Swap | Settings
 *
 * On launch, checks if a wallet exists:
 *   - No wallet -> WelcomeScreen (create or import)
 *   - Wallet exists -> UnlockScreen (PIN/biometric)
 */

import React, { useState, useEffect } from 'react';
import { ActivityIndicator, View, Text, StyleSheet } from 'react-native';
import { NavigationContainer } from '@react-navigation/native';
import { createNativeStackNavigator } from '@react-navigation/native-stack';
import { createBottomTabNavigator } from '@react-navigation/bottom-tabs';
import { keystore } from '../utils/keystore';

import WelcomeScreen from '../screens/WelcomeScreen';
import CreateWalletScreen from '../screens/CreateWalletScreen';
import ImportWalletScreen from '../screens/ImportWalletScreen';
import UnlockScreen from '../screens/UnlockScreen';
import HomeScreen from '../screens/HomeScreen';
import SendScreen from '../screens/SendScreen';
import ReceiveScreen from '../screens/ReceiveScreen';
import ObjectsScreen from '../screens/ObjectsScreen';
import ObjectDetailScreen from '../screens/ObjectDetailScreen';
import NftScreen from '../screens/NftScreen';
import NftDetailScreen from '../screens/NftDetailScreen';
import SwapScreen from '../screens/SwapScreen';
import SettingsScreen from '../screens/SettingsScreen';
import HistoryScreen from '../screens/HistoryScreen';
import FaucetScreen from '../screens/FaucetScreen';
import EnergyDashboardScreen from '../screens/EnergyDashboardScreen';

// ── Types ──

export type RootStackParamList = {
  Welcome: undefined;
  CreateWallet: undefined;
  ImportWallet: undefined;
  Unlock: undefined;
  MainTabs: undefined;
  Send: { prefillAddress?: string } | undefined;
  Receive: undefined;
  Objects: undefined;
  ObjectDetail: { objectId: string };
  NFTs: undefined;
  NftDetail: { nftId: string };
  History: undefined;
  Faucet: undefined;
};

export type TabParamList = {
  Home: undefined;
  EnergyDashboard: undefined;
  Swap: undefined;
  Settings: undefined;
};

// ── Tab Icon ──

const TabIcon: React.FC<{ label: string; color: string; focused: boolean }> = ({ label, color, focused }) => (
  <View style={[tabStyles.icon, focused && { backgroundColor: color + '18' }]}>
    <Text style={[tabStyles.iconText, { color }]}>{label}</Text>
  </View>
);

const tabStyles = StyleSheet.create({
  icon: {
    width: 32,
    height: 32,
    borderRadius: 10,
    alignItems: 'center',
    justifyContent: 'center',
  },
  iconText: {
    fontSize: 16,
    fontWeight: '700',
  },
});

// ── Bottom Tabs ──

const Tab = createBottomTabNavigator<TabParamList>();

const MainTabs: React.FC = () => (
  <Tab.Navigator
    screenOptions={{
      headerStyle: { backgroundColor: '#ffffff' },
      headerTintColor: '#111827',
      headerTitleStyle: { fontWeight: '600' },
      headerShadowVisible: false,
      tabBarStyle: {
        backgroundColor: '#ffffff',
        borderTopWidth: 1,
        borderTopColor: '#f3f4f6',
        paddingTop: 6,
        height: 84,
      },
      tabBarActiveTintColor: '#06b6d4',
      tabBarInactiveTintColor: '#9ca3af',
      tabBarLabelStyle: {
        fontSize: 11,
        fontWeight: '600',
      },
    }}
  >
    <Tab.Screen
      name="Home"
      component={HomeScreen}
      options={{
        title: 'EvaporChain',
        tabBarLabel: 'Home',
        tabBarIcon: ({ color, focused }) => <TabIcon label="H" color={color} focused={focused} />,
      }}
    />
    <Tab.Screen
      name="EnergyDashboard"
      component={EnergyDashboardScreen}
      options={{
        title: 'Energy',
        tabBarLabel: 'Energy',
        tabBarIcon: ({ color, focused }) => <TabIcon label="E" color={color} focused={focused} />,
      }}
    />
    <Tab.Screen
      name="Swap"
      component={SwapScreen}
      options={{
        title: 'Swap',
        tabBarLabel: 'Swap',
        tabBarIcon: ({ color, focused }) => <TabIcon label="S" color={color} focused={focused} />,
      }}
    />
    <Tab.Screen
      name="Settings"
      component={SettingsScreen}
      options={{
        title: 'Settings',
        tabBarLabel: 'Settings',
        tabBarIcon: ({ color, focused }) => <TabIcon label="G" color={color} focused={focused} />,
      }}
    />
  </Tab.Navigator>
);

// ── Root Stack ──

const Stack = createNativeStackNavigator<RootStackParamList>();

const headerStyle = { backgroundColor: '#ffffff' };
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
        <Stack.Screen name="Welcome" component={WelcomeScreen} options={{ headerShown: false }} />
        <Stack.Screen name="CreateWallet" component={CreateWalletScreen} options={{ title: 'Create Wallet' }} />
        <Stack.Screen name="ImportWallet" component={ImportWalletScreen} options={{ title: 'Import Wallet' }} />

        {/* Auth */}
        <Stack.Screen name="Unlock" component={UnlockScreen} options={{ headerShown: false }} />

        {/* Main App (tabs) */}
        <Stack.Screen
          name="MainTabs"
          component={MainTabs}
          options={{ headerShown: false, gestureEnabled: false }}
        />

        {/* Detail Screens (pushed on top of tabs) */}
        <Stack.Screen name="Send" component={SendScreen} options={{ title: 'Send EVAP' }} />
        <Stack.Screen name="Receive" component={ReceiveScreen} options={{ title: 'Receive EVAP' }} />
        <Stack.Screen name="Objects" component={ObjectsScreen} options={{ title: 'My Objects' }} />
        <Stack.Screen name="ObjectDetail" component={ObjectDetailScreen} options={{ title: 'Object' }} />
        <Stack.Screen name="NFTs" component={NftScreen} options={{ title: 'My NFTs' }} />
        <Stack.Screen name="NftDetail" component={NftDetailScreen} options={{ title: 'NFT' }} />
        <Stack.Screen name="History" component={HistoryScreen} options={{ title: 'Transaction History' }} />
        <Stack.Screen name="Faucet" component={FaucetScreen} options={{ title: 'Testnet Faucet' }} />
      </Stack.Navigator>
    </NavigationContainer>
  );
};

export default AppNavigator;
