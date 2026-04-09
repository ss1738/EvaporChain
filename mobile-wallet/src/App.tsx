/**
 * EvaporChain Mobile Wallet — Main App Entry
 *
 * Handles push notification registration and auto-lock enforcement.
 */

import React, { useEffect, useRef } from 'react';
import { StatusBar } from 'expo-status-bar';
import { SafeAreaProvider } from 'react-native-safe-area-context';
import { NavigationContainerRef } from '@react-navigation/native';
import AppNavigator from './navigation/AppNavigator';
import type { RootStackParamList } from './navigation/AppNavigator';
import { notifications } from './utils/notifications';
import { autoLockManager } from './utils/autolock';
import { NetworkBanner } from './components/NetworkBanner';

const App: React.FC = () => {
  const navigationRef = useRef<NavigationContainerRef<RootStackParamList>>(null);

  useEffect(() => {
    notifications.register().catch(console.warn);

    autoLockManager.start(() => {
      // Navigate to Unlock screen when auto-lock triggers
      if (navigationRef.current?.isReady()) {
        navigationRef.current.reset({
          index: 0,
          routes: [{ name: 'Unlock' }],
        });
      }
    });

    return () => autoLockManager.stop();
  }, []);

  return (
    <SafeAreaProvider>
      <StatusBar style="dark" />
      <NetworkBanner />
      <AppNavigator navigationRef={navigationRef} />
    </SafeAreaProvider>
  );
};

export default App;
