/**
 * Push Notification Manager for EvaporChain Wallet
 *
 * Handles:
 * - Decay warnings for objects and NFTs approaching Ghost state
 * - Incoming transfer notifications
 * - Registration with Expo push notification service
 */

import * as Notifications from 'expo-notifications';
import { Platform } from 'react-native';
import type { ChainObject, NFT } from './api';

// Configure notification handler
Notifications.setNotificationHandler({
  handleNotification: async () => ({
    shouldShowAlert: true,
    shouldPlaySound: true,
    shouldSetBadge: true,
  }),
});

export interface DecayWarning {
  objectId: string;
  name: string;
  hoursRemaining: number;
  type: 'object' | 'nft';
}

export const notifications = {
  /**
   * Request notification permissions and register for push.
   * Returns the Expo push token or null if denied.
   */
  async register(): Promise<string | null> {
    const { status: existingStatus } = await Notifications.getPermissionsAsync();
    let finalStatus = existingStatus;

    if (existingStatus !== 'granted') {
      const { status } = await Notifications.requestPermissionsAsync();
      finalStatus = status;
    }

    if (finalStatus !== 'granted') {
      return null;
    }

    // Android notification channel
    if (Platform.OS === 'android') {
      await Notifications.setNotificationChannelAsync('decay-warnings', {
        name: 'Decay Warnings',
        importance: Notifications.AndroidImportance.HIGH,
        vibrationPattern: [0, 250, 250, 250],
        lightColor: '#f59e0b',
      });

      await Notifications.setNotificationChannelAsync('transfers', {
        name: 'Transfers',
        importance: Notifications.AndroidImportance.HIGH,
        vibrationPattern: [0, 250],
        lightColor: '#22c55e',
      });
    }

    const token = await Notifications.getExpoPushTokenAsync();
    return token.data;
  },

  /**
   * Schedule a local decay warning notification.
   */
  async scheduleDecayWarning(warning: DecayWarning): Promise<string> {
    const triggerSeconds = Math.max(
      (warning.hoursRemaining - 2) * 3600,
      60 // At least 1 minute from now
    );

    const id = await Notifications.scheduleNotificationAsync({
      content: {
        title: 'Decay Warning',
        body: `Your ${warning.type === 'nft' ? 'NFT' : 'object'} "${warning.name}" has ${warning.hoursRemaining} hours before Ghost state. Refresh now to preserve it.`,
        data: {
          type: 'decay_warning',
          objectId: warning.objectId,
          objectType: warning.type,
        },
        sound: true,
        ...(Platform.OS === 'android' && { channelId: 'decay-warnings' }),
      },
      trigger: {
        type: Notifications.SchedulableTriggerInputTypes.TIME_INTERVAL,
        seconds: triggerSeconds,
      },
    });

    return id;
  },

  /**
   * Send an immediate notification for incoming transfer.
   */
  async notifyIncomingTransfer(
    fromAddress: string,
    amount: string
  ): Promise<void> {
    const shortAddr = `${fromAddress.slice(0, 6)}...${fromAddress.slice(-4)}`;

    await Notifications.scheduleNotificationAsync({
      content: {
        title: 'EVAP Received',
        body: `You received ${amount} EVAP from ${shortAddr}`,
        data: { type: 'incoming_transfer', from: fromAddress, amount },
        sound: true,
        ...(Platform.OS === 'android' && { channelId: 'transfers' }),
      },
      trigger: null, // Immediate
    });
  },

  /**
   * Scan owned objects/NFTs and schedule warnings for those
   * with less than 20% energy remaining.
   */
  async scheduleDecayWarningsForAssets(
    objects: ChainObject[],
    nfts: NFT[]
  ): Promise<void> {
    // Cancel existing decay warnings first
    await Notifications.cancelAllScheduledNotificationsAsync();

    const urgentObjects = objects
      .filter((obj) => obj.energy / obj.maxEnergy < 0.2 && obj.state !== 'Ghost')
      .map((obj) => ({
        objectId: obj.id,
        name: obj.name,
        hoursRemaining: Math.max(
          0,
          (obj.estimatedGhostTime - Date.now()) / 3600000
        ),
        type: 'object' as const,
      }));

    const urgentNfts = nfts
      .filter((nft) => nft.energy / nft.maxEnergy < 0.2 && nft.state !== 'Ghost')
      .map((nft) => ({
        objectId: nft.id,
        name: nft.name,
        hoursRemaining: Math.max(
          0,
          (nft.estimatedGhostTime - Date.now()) / 3600000
        ),
        type: 'nft' as const,
      }));

    const allWarnings = [...urgentObjects, ...urgentNfts].sort(
      (a, b) => a.hoursRemaining - b.hoursRemaining
    );

    // Schedule top 10 most urgent
    for (const warning of allWarnings.slice(0, 10)) {
      await this.scheduleDecayWarning(warning);
    }
  },

  /**
   * Add a listener for notification taps.
   */
  addResponseListener(
    callback: (response: Notifications.NotificationResponse) => void
  ): Notifications.Subscription {
    return Notifications.addNotificationResponseReceivedListener(callback);
  },

  /**
   * Add a listener for received notifications (foreground).
   */
  addReceivedListener(
    callback: (notification: Notifications.Notification) => void
  ): Notifications.Subscription {
    return Notifications.addNotificationReceivedListener(callback);
  },
};

export default notifications;
