/**
 * Auto-Lock Manager
 *
 * Tracks app background/foreground transitions and triggers lock
 * when the configured timeout is exceeded.
 */

import { AppState, type AppStateStatus } from 'react-native';
import { keystore } from './keystore';

type LockCallback = () => void;

class AutoLockManager {
  private backgroundTimestamp: number | null = null;
  private listener: ReturnType<typeof AppState.addEventListener> | null = null;
  private onLock: LockCallback | null = null;

  start(onLock: LockCallback): void {
    this.onLock = onLock;
    this.listener = AppState.addEventListener('change', this.handleAppStateChange);
  }

  stop(): void {
    this.listener?.remove();
    this.listener = null;
    this.onLock = null;
    this.backgroundTimestamp = null;
  }

  private handleAppStateChange = async (nextState: AppStateStatus) => {
    if (nextState === 'background' || nextState === 'inactive') {
      this.backgroundTimestamp = Date.now();
    } else if (nextState === 'active' && this.backgroundTimestamp) {
      const elapsed = Date.now() - this.backgroundTimestamp;
      const timeoutMinutes = await keystore.getAutoLockTimeout();
      const timeoutMs = timeoutMinutes * 60 * 1000;

      if (elapsed >= timeoutMs && this.onLock) {
        this.onLock();
      }

      this.backgroundTimestamp = null;
    }
  };
}

export const autoLockManager = new AutoLockManager();
