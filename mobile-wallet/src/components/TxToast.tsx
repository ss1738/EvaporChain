/**
 * Transient bottom-of-screen toast surface for tx finalisation.
 *
 * Mounted once at the navigator root (App.tsx). Reads the toast queue
 * from `useToasts()`; each toast slides up on mount and auto-dismisses
 * 4 seconds later. Green for finalised, red for rejected. Tx hash chip
 * is click-to-copy via `expo-clipboard`.
 *
 * Mirrors `extension/src/components/TxToast.tsx`, adapted to React
 * Native: the slide animation uses `Animated.spring` (translateY), and
 * positioning respects the bottom safe-area inset so the toast never
 * collides with the home indicator on iPhone or the gesture bar on
 * Android.
 */

import React, { useEffect, useRef, useState } from 'react';
import {
  View,
  Text,
  TouchableOpacity,
  StyleSheet,
  Animated,
  Easing,
} from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import * as Clipboard from 'expo-clipboard';
import { useToasts, useTxStore, type Toast } from '../state/txStore';

const AUTO_DISMISS_MS = 4000;

export const TxToastContainer: React.FC = () => {
  const toasts = useToasts();
  const insets = useSafeAreaInsets();

  if (toasts.length === 0) return null;

  return (
    <View
      pointerEvents="box-none"
      style={[
        styles.container,
        { bottom: Math.max(12, insets.bottom + 8) },
      ]}
    >
      {toasts.map((t) => (
        <TxToast key={t.id} toast={t} />
      ))}
    </View>
  );
};

const TxToast: React.FC<{ toast: Toast }> = ({ toast }) => {
  const { dismissToast } = useTxStore();
  const translateY = useRef(new Animated.Value(60)).current;
  const opacity = useRef(new Animated.Value(0)).current;
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    Animated.parallel([
      Animated.spring(translateY, {
        toValue: 0,
        useNativeDriver: true,
        speed: 18,
        bounciness: 6,
      }),
      Animated.timing(opacity, {
        toValue: 1,
        duration: 180,
        easing: Easing.out(Easing.quad),
        useNativeDriver: true,
      }),
    ]).start();

    const dismissId = setTimeout(() => {
      Animated.parallel([
        Animated.timing(translateY, {
          toValue: 80,
          duration: 180,
          useNativeDriver: true,
        }),
        Animated.timing(opacity, {
          toValue: 0,
          duration: 180,
          useNativeDriver: true,
        }),
      ]).start(() => dismissToast(toast.id));
    }, AUTO_DISMISS_MS);

    return () => clearTimeout(dismissId);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [toast.id]);

  const isFinalised = toast.kind === 'finalised';
  const palette = isFinalised
    ? { border: '#86efac', bg: '#f0fdf4', accent: '#16a34a' }
    : { border: '#fca5a5', bg: '#fef2f2', accent: '#dc2626' };
  const icon = isFinalised ? '✓' : '✗';
  const label = isFinalised ? 'Tx finalised' : 'Tx rejected';

  const shortHash = toast.hash.length > 14
    ? `${toast.hash.slice(0, 8)}…${toast.hash.slice(-6)}`
    : toast.hash;

  const handleCopy = async () => {
    try {
      await Clipboard.setStringAsync(toast.hash);
      setCopied(true);
      setTimeout(() => setCopied(false), 1200);
    } catch {
      // Clipboard unavailable — silently ignore.
    }
  };

  return (
    <Animated.View
      style={[
        styles.toast,
        { borderColor: palette.border, backgroundColor: palette.bg },
        { transform: [{ translateY }], opacity },
      ]}
    >
      <Text style={[styles.icon, { color: palette.accent }]}>{icon}</Text>
      <View style={styles.body}>
        <Text style={styles.label} numberOfLines={1}>
          <Text style={styles.labelStrong}>{label}</Text>
          <Text style={styles.labelDim}>{` — ${toast.summary}`}</Text>
        </Text>
        <TouchableOpacity
          onPress={handleCopy}
          style={[styles.chip, { borderColor: palette.border }]}
          activeOpacity={0.7}
        >
          <Text style={[styles.chipText, { color: palette.accent }]}>
            {shortHash}
          </Text>
          <Text style={styles.chipHint}>{copied ? 'copied' : 'copy'}</Text>
        </TouchableOpacity>
      </View>
      <TouchableOpacity
        onPress={() => dismissToast(toast.id)}
        style={styles.close}
        activeOpacity={0.6}
        hitSlop={{ top: 8, bottom: 8, left: 8, right: 8 }}
      >
        <Text style={styles.closeText}>×</Text>
      </TouchableOpacity>
    </Animated.View>
  );
};

const styles = StyleSheet.create({
  container: {
    position: 'absolute',
    left: 0,
    right: 0,
    alignItems: 'center',
    gap: 8,
    zIndex: 9999,
    elevation: 10,
  },
  toast: {
    flexDirection: 'row',
    alignItems: 'flex-start',
    width: '92%',
    maxWidth: 360,
    paddingHorizontal: 12,
    paddingVertical: 10,
    borderRadius: 12,
    borderWidth: 1,
    shadowColor: '#000',
    shadowOffset: { width: 0, height: 4 },
    shadowOpacity: 0.12,
    shadowRadius: 8,
    elevation: 6,
    gap: 10,
  },
  icon: {
    fontSize: 16,
    fontWeight: '700',
    marginTop: 1,
  },
  body: {
    flex: 1,
    minWidth: 0,
  },
  label: {
    fontSize: 12,
    color: '#374151',
  },
  labelStrong: {
    fontWeight: '700',
    color: '#111827',
  },
  labelDim: {
    color: '#6b7280',
  },
  chip: {
    flexDirection: 'row',
    alignSelf: 'flex-start',
    alignItems: 'center',
    gap: 6,
    marginTop: 6,
    paddingHorizontal: 8,
    paddingVertical: 3,
    borderRadius: 8,
    borderWidth: 1,
    backgroundColor: '#ffffff',
  },
  chipText: {
    fontSize: 10,
    fontFamily: 'monospace',
    fontWeight: '600',
  },
  chipHint: {
    fontSize: 9,
    color: '#9ca3af',
    textTransform: 'uppercase',
    letterSpacing: 0.4,
  },
  close: {
    paddingHorizontal: 4,
    paddingVertical: 0,
  },
  closeText: {
    fontSize: 18,
    color: '#9ca3af',
    fontWeight: '600',
    lineHeight: 18,
  },
});

export default TxToastContainer;
