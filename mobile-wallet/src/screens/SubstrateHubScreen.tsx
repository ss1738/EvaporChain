/**
 * SubstrateHubScreen — Index of substrate-visibility surfaces.
 *
 * Single entry point that lists the nine substrate primitives
 * (Patronage, Refresh-pool, Fee-controller, Demurrage, Governance,
 * HLWA, DSN, Bell-Beacon, LAD-VM) — each pushes the corresponding
 * detail screen onto the stack. Mirrors the browser extension's
 * substrate surfaces but adapted to mobile's screen-driven navigation.
 */

import React from 'react';
import {
  View,
  Text,
  TouchableOpacity,
  ScrollView,
  StyleSheet,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { useNavigation } from '@react-navigation/native';
import type { NativeStackNavigationProp } from '@react-navigation/native-stack';
import type { RootStackParamList } from '../navigation/AppNavigator';

type SubstrateRoute =
  | 'Patronage'
  | 'RefreshPool'
  | 'FeeController'
  | 'Demurrage'
  | 'Governance'
  | 'Hlwa'
  | 'Dsn'
  | 'BellBeacon'
  | 'LadVm'
  | 'HotColdStake';

interface Entry {
  title: string;
  blurb: string;
  badge: string;
  badgeColor: string;
  badgeBg: string;
  badgeBorder: string;
  screen: SubstrateRoute;
}

const ENTRIES: Entry[] = [
  {
    title: 'Patronage',
    blurb: 'Pre-fund eviction immunity for your objects.',
    badge: 'P',
    badgeColor: '#06b6d4',
    badgeBg: '#ecfeff',
    badgeBorder: '#a5f3fc',
    screen: 'Patronage',
  },
  {
    title: 'Refresh Pool',
    blurb: 'Where demurrage and patronage payments accumulate.',
    badge: 'R',
    badgeColor: '#22c55e',
    badgeBg: '#ecfdf5',
    badgeBorder: '#a7f3d0',
    screen: 'RefreshPool',
  },
  {
    title: 'Fee Controller',
    blurb: 'Lyapunov-stable base fee + pressure pill.',
    badge: 'F',
    badgeColor: '#f59e0b',
    badgeBg: '#fffbeb',
    badgeBorder: '#fde68a',
    screen: 'FeeController',
  },
  {
    title: 'Demurrage',
    blurb: 'Idle-balance fee owed at the current epoch.',
    badge: 'D',
    badgeColor: '#ef4444',
    badgeBg: '#fef2f2',
    badgeBorder: '#fecaca',
    screen: 'Demurrage',
  },
  {
    title: 'Governance',
    blurb: 'Fork-choice mode and Singh-Attractor basin set.',
    badge: 'G',
    badgeColor: '#8b5cf6',
    badgeBg: '#f5f3ff',
    badgeBorder: '#c4b5fd',
    screen: 'Governance',
  },
  {
    title: 'HLWA',
    blurb: 'Half-Life Wrapped Asset effective-supply decay.',
    badge: 'H',
    badgeColor: '#0e7490',
    badgeBg: '#ecfeff',
    badgeBorder: '#a5f3fc',
    screen: 'Hlwa',
  },
  {
    title: 'DSN',
    blurb: 'Decay-Stamped Nullifiers privacy window.',
    badge: 'N',
    badgeColor: '#8b5cf6',
    badgeBg: '#f5f3ff',
    badgeBorder: '#c4b5fd',
    screen: 'Dsn',
  },
  {
    title: 'Bell-Beacon',
    blurb: 'CHSH quantum-randomness certification.',
    badge: 'B',
    badgeColor: '#06b6d4',
    badgeBg: '#ecfeff',
    badgeBorder: '#a5f3fc',
    screen: 'BellBeacon',
  },
  {
    title: 'LAD-VM',
    blurb: 'Linear/Affine/Decaying substructural-resource simulator.',
    badge: 'L',
    badgeColor: '#7c3aed',
    badgeBg: '#f5f3ff',
    badgeBorder: '#c4b5fd',
    screen: 'LadVm',
  },
  {
    title: 'Hot-Cold Stake',
    blurb: 'Two-temperature validator stake equilibrium.',
    badge: 'H',
    badgeColor: '#f59e0b',
    badgeBg: '#fffbeb',
    badgeBorder: '#fde68a',
    screen: 'HotColdStake',
  },
];

const SubstrateHubScreen: React.FC = () => {
  const navigation = useNavigation<NativeStackNavigationProp<RootStackParamList>>();

  return (
    <SafeAreaView style={styles.container} edges={['bottom']}>
      <ScrollView contentContainerStyle={styles.scroll}>
        <Text style={styles.intro}>
          Substrate-visibility surfaces — the chain primitives the wallet exposes for inspection
          and direct interaction.
        </Text>
        {ENTRIES.map((e) => (
          <TouchableOpacity
            key={e.screen}
            style={styles.row}
            onPress={() => navigation.navigate(e.screen as never)}
            activeOpacity={0.7}
          >
            <View
              style={[
                styles.badge,
                { backgroundColor: e.badgeBg, borderColor: e.badgeBorder },
              ]}
            >
              <Text style={[styles.badgeText, { color: e.badgeColor }]}>{e.badge}</Text>
            </View>
            <View style={{ flex: 1, marginLeft: 12 }}>
              <Text style={styles.rowTitle}>{e.title}</Text>
              <Text style={styles.rowBlurb}>{e.blurb}</Text>
            </View>
            <Text style={styles.arrow}>{'>'}</Text>
          </TouchableOpacity>
        ))}
      </ScrollView>
    </SafeAreaView>
  );
};

const styles = StyleSheet.create({
  container: { flex: 1, backgroundColor: '#f9fafb' },
  scroll: { padding: 16, paddingBottom: 40 },
  intro: { fontSize: 12, color: '#6b7280', marginBottom: 16, lineHeight: 16 },
  row: {
    flexDirection: 'row',
    alignItems: 'center',
    backgroundColor: '#ffffff',
    borderRadius: 12,
    padding: 14,
    marginBottom: 8,
    borderWidth: 1,
    borderColor: '#e5e7eb',
    minHeight: 60,
  },
  badge: {
    width: 36,
    height: 36,
    borderRadius: 10,
    alignItems: 'center',
    justifyContent: 'center',
    borderWidth: 1,
  },
  badgeText: { fontSize: 14, fontWeight: '700' },
  rowTitle: { fontSize: 14, fontWeight: '600', color: '#111827' },
  rowBlurb: { fontSize: 11, color: '#9ca3af', marginTop: 2 },
  arrow: { fontSize: 16, color: '#9ca3af', fontWeight: '600' },
});

export default SubstrateHubScreen;
