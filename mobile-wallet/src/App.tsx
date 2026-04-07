/**
 * EvaporChain Mobile Wallet — Main App Entry
 */

import React, { useEffect } from 'react';
import { StatusBar } from 'expo-status-bar';
import { SafeAreaProvider } from 'react-native-safe-area-context';
import AppNavigator from './navigation/AppNavigator';
import { notifications } from './utils/notifications';

const App: React.FC = () => {
  useEffect(() => {
    // Register push notifications on launch
    notifications.register().catch(console.warn);
  }, []);

  return (
    <SafeAreaProvider>
      <StatusBar style="dark" />
      <AppNavigator />
    </SafeAreaProvider>
  );
};

export default App;
