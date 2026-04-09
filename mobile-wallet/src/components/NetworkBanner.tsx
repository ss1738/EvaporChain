/**
 * NetworkBanner — Shows connectivity status
 *
 * Displays a banner when the API is unreachable or the device is offline.
 * Auto-retries and dismisses when connection is restored.
 */

import React, { useState, useEffect } from 'react';
import { View, Text, StyleSheet, Animated } from 'react-native';
import { api } from '../utils/api';

type Status = 'connected' | 'checking' | 'offline';

export const NetworkBanner: React.FC = () => {
  const [status, setStatus] = useState<Status>('connected');
  const [opacity] = useState(new Animated.Value(0));

  useEffect(() => {
    let mounted = true;
    let retryTimer: ReturnType<typeof setTimeout>;

    const checkConnection = async () => {
      if (!mounted) return;
      setStatus('checking');
      try {
        await api.getChainStatus();
        if (mounted) setStatus('connected');
      } catch {
        if (mounted) {
          setStatus('offline');
          // Retry every 10 seconds
          retryTimer = setTimeout(checkConnection, 10000);
        }
      }
    };

    checkConnection();

    // Also check periodically
    const interval = setInterval(checkConnection, 30000);

    return () => {
      mounted = false;
      clearInterval(interval);
      clearTimeout(retryTimer);
    };
  }, []);

  useEffect(() => {
    Animated.timing(opacity, {
      toValue: status === 'offline' ? 1 : 0,
      duration: 300,
      useNativeDriver: true,
    }).start();
  }, [status, opacity]);

  if (status === 'connected') return null;

  return (
    <Animated.View style={[styles.container, { opacity }]}>
      <View style={styles.dot} />
      <Text style={styles.text}>
        {status === 'offline'
          ? 'Unable to reach EvaporChain network'
          : 'Reconnecting...'}
      </Text>
    </Animated.View>
  );
};

const styles = StyleSheet.create({
  container: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'center',
    backgroundColor: '#fef2f2',
    borderBottomWidth: 1,
    borderBottomColor: '#fecaca',
    paddingVertical: 8,
    paddingHorizontal: 16,
    gap: 8,
  },
  dot: {
    width: 8,
    height: 8,
    borderRadius: 4,
    backgroundColor: '#ef4444',
  },
  text: {
    fontSize: 13,
    color: '#dc2626',
    fontWeight: '500',
  },
});

export default NetworkBanner;
