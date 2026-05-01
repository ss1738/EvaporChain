/**
 * EvaporChain Mobile Wallet — Main App Entry
 *
 * Handles push notification registration and auto-lock enforcement.
 * Mounts the TxStoreProvider so any screen can call trackTx() after a
 * broadcast, and the TxToastContainer at the root so finalisation
 * toasts are visible across all screens.
 */

// MUST be the very first import — installs a native crypto.getRandomValues
// shim before any module that touches @noble/* / ml-dsa / mnemonic code.
import 'react-native-get-random-values';

import React, { useEffect, useRef } from 'react';
import { StatusBar } from 'expo-status-bar';
import { SafeAreaProvider } from 'react-native-safe-area-context';
import { NavigationContainerRef } from '@react-navigation/native';
import AppNavigator from './navigation/AppNavigator';
import type { RootStackParamList } from './navigation/AppNavigator';
import { notifications } from './utils/notifications';
import { autoLockManager } from './utils/autolock';
import { txTracker } from './utils/txTracker';
import { NetworkBanner } from './components/NetworkBanner';
import { TxToastContainer } from './components/TxToast';
import { TxStoreProvider } from './state/txStore';

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

    // Tx-status polling. The tracker is idle while pendingTxs is empty
    // and pauses entirely while the app is backgrounded — see
    // utils/txTracker.ts for the AppState wiring.
    txTracker.start();

    return () => {
      autoLockManager.stop();
      txTracker.stop();
    };
  }, []);

  return (
    <SafeAreaProvider>
      <TxStoreProvider>
        <StatusBar style="dark" />
        <NetworkBanner />
        <AppNavigator navigationRef={navigationRef} />
        <TxToastContainer />
      </TxStoreProvider>
    </SafeAreaProvider>
  );
};

export default App;
