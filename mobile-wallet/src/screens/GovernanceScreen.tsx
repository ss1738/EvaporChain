/**
 * GovernanceScreen — Fork-choice mode + attractor set.
 *
 * Mirrors extension/src/components/GovernanceScreen.tsx.
 *
 *   GET  /api/governance/fork_choice_mode  — current mode + attractors
 *   POST /api/governance/fork_choice_mode  — stake-quorum amendment
 *
 * The endorse form is gated behind the React Native `__DEV__` global
 * (Metro defines this; production bundles strip the form). The chain
 * does NOT take a signature for this endpoint — it enforces quorum from
 * raw stake numbers. We still display the form behind dev gating.
 */

import React, { useCallback, useEffect, useState } from 'react';
import {
  View,
  Text,
  TextInput,
  TouchableOpacity,
  ScrollView,
  StyleSheet,
  ActivityIndicator,
  Alert,
  RefreshControl,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { api } from '../utils/api';
import type { ForkChoiceAttractor, ForkChoiceModeStatus } from '../utils/api';

const GovernanceScreen: React.FC = () => {
  const [mode, setMode] = useState<ForkChoiceModeStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [submitting, setSubmitting] = useState(false);
  const [targetMode, setTargetMode] = useState<string>('singh_attractor');
  const [attractorJson, setAttractorJson] = useState<string>(
    JSON.stringify([{ center: 1000, basin_radius: 200 }], null, 2),
  );
  const [stake, setStake] = useState<string>('1000');
  const [requiredStake, setRequiredStake] = useState<string>('1500');

  // RN dev gate — Metro defines __DEV__ globally. true in dev/Expo Go.
  const proGated = typeof __DEV__ !== 'undefined' ? __DEV__ : false;

  const load = useCallback(async () => {
    try {
      const m = await api.getForkChoiceMode();
      setMode(m);
    } catch {
      /* ignore */
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const handleEndorse = async () => {
    let attractors: ForkChoiceAttractor[] | undefined;
    try {
      const parsed = JSON.parse(attractorJson);
      if (!Array.isArray(parsed)) throw new Error('attractors must be a JSON array');
      attractors = parsed.map((a: { center: number; basin_radius: number }) => ({
        center: Number(a.center),
        basin_radius: Number(a.basin_radius),
      }));
    } catch (e) {
      Alert.alert('Invalid JSON', String(e));
      return;
    }

    const stakeN = parseInt(stake, 10);
    const reqN = parseInt(requiredStake, 10);
    if (!Number.isFinite(stakeN) || stakeN <= 0) {
      Alert.alert('Invalid', 'Stake must be > 0');
      return;
    }
    if (!Number.isFinite(reqN) || reqN <= 0) {
      Alert.alert('Invalid', 'Required stake must be > 0');
      return;
    }

    setSubmitting(true);
    try {
      const resp = await api.amendForkChoiceMode({
        mode: targetMode,
        attractors: targetMode === 'singh_attractor' ? attractors : undefined,
        endorser_stakes: [stakeN],
        required_stake: reqN,
      });
      if (resp.status === 'amended') {
        Alert.alert('Amended', `Fork-choice → ${resp.fork_choice_mode}`);
        await load();
      } else {
        Alert.alert('Rejected', resp.detail || 'Amendment rejected');
      }
    } catch (e) {
      Alert.alert('Error', String(e));
    } finally {
      setSubmitting(false);
    }
  };

  if (loading) {
    return (
      <View style={styles.centered}>
        <ActivityIndicator size="large" color="#06b6d4" />
      </View>
    );
  }

  const currentMode = mode?.fork_choice_mode ?? '—';
  const attractors = mode?.attractors ?? [];

  return (
    <SafeAreaView style={styles.container} edges={['bottom']}>
      <ScrollView
        contentContainerStyle={styles.scroll}
        refreshControl={<RefreshControl refreshing={false} onRefresh={load} tintColor="#06b6d4" />}
      >
        <View style={styles.card}>
          <Text style={styles.label}>Active fork-choice mode</Text>
          <Text style={styles.bigVal}>
            {currentMode === 'mcc'
              ? 'MCC'
              : currentMode === 'singh_attractor'
              ? 'Singh-Attractor'
              : currentMode}
          </Text>
          {mode?.detail ? <Text style={styles.detail}>{mode.detail}</Text> : null}
        </View>

        <ModeCard
          name="MCC"
          id="mcc"
          active={currentMode === 'mcc'}
          blurb="Most-Cumulative-Choice — vanilla heaviest-chain selector. Default genesis rule."
        />
        <ModeCard
          name="Singh-Attractor"
          id="singh_attractor"
          active={currentMode === 'singh_attractor'}
          blurb="Pulls competing forks toward attractor centres weighted by basin_radius — converges to a unique tip when basins are non-overlapping."
        />

        {currentMode === 'singh_attractor' ? (
          <View style={styles.card}>
            <Text style={styles.label}>Attractor set ({attractors.length})</Text>
            {attractors.length === 0 ? (
              <Text style={styles.muted}>No attractors configured.</Text>
            ) : (
              attractors.map((a, i) => (
                <View key={i} style={styles.attractorRow}>
                  <Text style={styles.kvKey}>#{i + 1}</Text>
                  <Text style={styles.kvVal}>
                    centre {a.center.toLocaleString()} · basin {a.basin_radius.toLocaleString()}
                  </Text>
                </View>
              ))
            )}
          </View>
        ) : null}

        <View style={styles.card}>
          <View style={styles.proHeader}>
            <Text style={styles.label}>Endorse amendment</Text>
            <View style={styles.proPill}>
              <Text style={styles.proPillText}>Pro</Text>
            </View>
          </View>
          {!proGated ? (
            <Text style={styles.muted}>
              Amendment endorsement requires a dev build. Heavy stake-weighted op.
            </Text>
          ) : (
            <View>
              <Text style={styles.fieldLabel}>Target mode</Text>
              <View style={styles.modeRow}>
                {(['mcc', 'singh_attractor'] as const).map((m) => (
                  <TouchableOpacity
                    key={m}
                    style={[styles.modeOpt, targetMode === m && styles.modeOptActive]}
                    onPress={() => setTargetMode(m)}
                    activeOpacity={0.7}
                  >
                    <Text style={[styles.modeOptText, targetMode === m && styles.modeOptTextActive]}>
                      {m === 'mcc' ? 'MCC' : 'Singh'}
                    </Text>
                  </TouchableOpacity>
                ))}
              </View>

              <Text style={styles.fieldLabel}>Attractors (JSON)</Text>
              <TextInput
                style={[styles.input, styles.inputMono, { minHeight: 80 }]}
                value={attractorJson}
                onChangeText={setAttractorJson}
                multiline
                autoCapitalize="none"
              />

              <View style={styles.gridRow}>
                <View style={{ flex: 1 }}>
                  <Text style={styles.fieldLabel}>Your stake</Text>
                  <TextInput
                    style={styles.input}
                    value={stake}
                    onChangeText={setStake}
                    keyboardType="numeric"
                  />
                </View>
                <View style={{ flex: 1 }}>
                  <Text style={styles.fieldLabel}>Required quorum</Text>
                  <TextInput
                    style={styles.input}
                    value={requiredStake}
                    onChangeText={setRequiredStake}
                    keyboardType="numeric"
                  />
                </View>
              </View>

              <TouchableOpacity
                style={[styles.broadcastBtn, submitting && { opacity: 0.5 }]}
                disabled={submitting}
                onPress={handleEndorse}
                activeOpacity={0.7}
              >
                {submitting ? (
                  <ActivityIndicator color="#ffffff" />
                ) : (
                  <Text style={styles.broadcastBtnText}>Broadcast amendment</Text>
                )}
              </TouchableOpacity>
              <Text style={styles.muted}>
                The chain enforces quorum from raw stakes — no signature required.
              </Text>
            </View>
          )}
        </View>
      </ScrollView>
    </SafeAreaView>
  );
};

const ModeCard: React.FC<{ name: string; id: string; active: boolean; blurb: string }> = ({
  name,
  id,
  active,
  blurb,
}) => (
  <View style={[styles.card, active && styles.cardActive]}>
    <View style={styles.modeHeader}>
      <Text style={styles.modeName}>{name}</Text>
      {active ? (
        <View style={styles.activePill}>
          <Text style={styles.activePillText}>Active</Text>
        </View>
      ) : (
        <Text style={styles.modeId}>{id}</Text>
      )}
    </View>
    <Text style={styles.blurb}>{blurb}</Text>
  </View>
);

const styles = StyleSheet.create({
  container: { flex: 1, backgroundColor: '#f9fafb' },
  centered: { flex: 1, alignItems: 'center', justifyContent: 'center', backgroundColor: '#f9fafb' },
  scroll: { padding: 16, paddingBottom: 40 },
  card: {
    backgroundColor: '#ffffff',
    borderRadius: 14,
    padding: 16,
    marginBottom: 12,
    borderWidth: 1,
    borderColor: '#e5e7eb',
  },
  cardActive: { backgroundColor: '#ecfeff', borderColor: '#67e8f9' },
  label: {
    fontSize: 11,
    color: '#6b7280',
    fontWeight: '600',
    textTransform: 'uppercase',
    letterSpacing: 0.5,
    marginBottom: 6,
  },
  bigVal: { fontSize: 24, fontWeight: '700', color: '#06b6d4' },
  detail: { fontSize: 11, color: '#6b7280', marginTop: 6, lineHeight: 15 },
  muted: { fontSize: 12, color: '#9ca3af' },
  modeHeader: { flexDirection: 'row', justifyContent: 'space-between', alignItems: 'center' },
  modeName: { fontSize: 14, fontWeight: '700', color: '#111827' },
  modeId: { fontSize: 10, color: '#9ca3af', fontFamily: 'monospace' },
  activePill: {
    backgroundColor: '#ecfeff',
    borderColor: '#67e8f9',
    borderWidth: 1,
    borderRadius: 999,
    paddingHorizontal: 8,
    paddingVertical: 2,
  },
  activePillText: { color: '#06b6d4', fontSize: 10, fontWeight: '600' },
  blurb: { fontSize: 11, color: '#6b7280', marginTop: 6, lineHeight: 15 },
  attractorRow: {
    flexDirection: 'row',
    justifyContent: 'space-between',
    paddingVertical: 6,
  },
  kvKey: { fontSize: 12, color: '#6b7280' },
  kvVal: { fontSize: 13, color: '#111827', fontWeight: '600' },
  proHeader: { flexDirection: 'row', justifyContent: 'space-between', alignItems: 'center' },
  proPill: {
    backgroundColor: '#f5f3ff',
    borderColor: '#c4b5fd',
    borderWidth: 1,
    borderRadius: 999,
    paddingHorizontal: 8,
    paddingVertical: 2,
  },
  proPillText: { color: '#8b5cf6', fontSize: 10, fontWeight: '700' },
  fieldLabel: {
    fontSize: 10,
    color: '#9ca3af',
    fontWeight: '600',
    textTransform: 'uppercase',
    letterSpacing: 0.4,
    marginTop: 8,
    marginBottom: 4,
  },
  modeRow: { flexDirection: 'row', gap: 6 },
  modeOpt: {
    flex: 1,
    backgroundColor: '#f9fafb',
    borderWidth: 1,
    borderColor: '#e5e7eb',
    borderRadius: 8,
    paddingVertical: 10,
    alignItems: 'center',
  },
  modeOptActive: { backgroundColor: '#06b6d4', borderColor: '#06b6d4' },
  modeOptText: { fontSize: 12, fontWeight: '600', color: '#6b7280' },
  modeOptTextActive: { color: '#ffffff' },
  input: {
    backgroundColor: '#f9fafb',
    borderWidth: 1,
    borderColor: '#e5e7eb',
    borderRadius: 8,
    paddingHorizontal: 12,
    paddingVertical: 8,
    fontSize: 13,
    color: '#111827',
  },
  inputMono: { fontFamily: 'monospace', fontSize: 11, textAlignVertical: 'top' },
  gridRow: { flexDirection: 'row', gap: 8 },
  broadcastBtn: {
    backgroundColor: '#06b6d4',
    borderRadius: 12,
    paddingVertical: 12,
    alignItems: 'center',
    marginTop: 12,
    marginBottom: 6,
    minHeight: 44,
    justifyContent: 'center',
  },
  broadcastBtnText: { color: '#ffffff', fontSize: 14, fontWeight: '700' },
});

export default GovernanceScreen;
