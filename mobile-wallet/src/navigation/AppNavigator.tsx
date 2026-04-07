/**
 * AppNavigator — React Navigation stack configuration
 *
 * Stack: Unlock -> Home -> Send / Receive / Objects / NFTs / Swap / Settings
 */

import React from 'react';
import { NavigationContainer } from '@react-navigation/native';
import { createNativeStackNavigator } from '@react-navigation/native-stack';

import UnlockScreen from '../screens/UnlockScreen';
import HomeScreen from '../screens/HomeScreen';
import SendScreen from '../screens/SendScreen';
import ReceiveScreen from '../screens/ReceiveScreen';
import ObjectsScreen from '../screens/ObjectsScreen';
import NftScreen from '../screens/NftScreen';
import SwapScreen from '../screens/SwapScreen';
import SettingsScreen from '../screens/SettingsScreen';

export type RootStackParamList = {
  Unlock: undefined;
  Home: undefined;
  Send: { prefillAddress?: string } | undefined;
  Receive: undefined;
  Objects: undefined;
  NFTs: undefined;
  Swap: undefined;
  Settings: undefined;
};

const Stack = createNativeStackNavigator<RootStackParamList>();

const headerStyle = {
  backgroundColor: '#ffffff',
};

const headerTintColor = '#111827';

export const AppNavigator: React.FC = () => {
  return (
    <NavigationContainer>
      <Stack.Navigator
        initialRouteName="Unlock"
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
        <Stack.Screen
          name="Unlock"
          component={UnlockScreen}
          options={{ headerShown: false }}
        />
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
          name="Settings"
          component={SettingsScreen}
          options={{ title: 'Settings' }}
        />
      </Stack.Navigator>
    </NavigationContainer>
  );
};

export default AppNavigator;
