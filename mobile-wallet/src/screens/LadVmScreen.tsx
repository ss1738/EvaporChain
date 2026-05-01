/**
 * LadVmScreen — LAD-VM substructural-resource lifecycle simulator.
 *
 * Mirrors `extension/src/components/LadVmPreview.tsx`, with the
 * extension SendScreen's target-object selector folded into the same
 * surface so the mobile flow is one screen instead of two.
 *
 *   Mode toggle: "Address" / "Object"
 *     Address mode → free-form mode/value/decay window inputs.
 *     Object mode  → dropdown of owned objects with `isLadTyped === true`.
 *                    Picking an object pre-fills the LAD mode from
 *                    `object.ladMode`. Non-LAD-typed objects are still
 *                    shown but disabled in the picker so the user
 *                    understands why they can't simulate against them.
 *
 * Endpoint: POST /api/lad_vm/simulate (api.rs §post_lad_vm_simulate)
 *
 * RN component choice: the dropdown is implemented as a custom
 * full-screen `Modal` listing the owned objects rather than the
 * platform-native `<Picker>` (which is awkwardly small on iOS and
 * doesn't render LAD-mode badges). This keeps zero new dependencies.
 */

import React, { useEffect, useMemo, useState } from 'react';
import {
  View,
  Text,
  TextInput,
  TouchableOpacity,
  ScrollView,
  StyleSheet,
  ActivityIndicator,
  Modal,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { api } from '../utils/api';
import type {
  ChainObject,
  ChainStatus,
  LadAction,
  LadMode,
  LadSimResp,
} from '../utils/api';
import { keystore } from '../utils/keystore';

type Source = 'address' | 'object';

const MODES: LadMode[] = ['linear', 'affine', 'decaying'];
const ACTIONS: LadAction[] = ['use', 'drop', 'tick'];

const LadVmScreen: React.FC = () => {
  const [chainStatus, setChainStatus] = useState<ChainStatus | null>(null);
  const [objects, setObjects] = useState<ChainObject[]>([]);
  const [source, setSource] = useState<Source>('address');
  const [pickedObjectId, setPickedObjectId] = useState<string>('');
  const [pickerOpen, setPickerOpen] = useState(false);

  const [mode, setMode] = useState<LadMode>('decaying');
  const [action, setAction] = useState<LadAction>('use');
  const [value, setValue] = useState('42');
  const [createdAtEpoch, setCreatedAtEpoch] = useState('0');
  const [decayWindow, setDecayWindow] = useState('10');
  const [currentEpochInput, setCurrentEpochInput] = useState('0');

  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<LadSimResp | null>(null);

  // Load chain status + owned objects once.
  useEffect(() => {
    void (async () => {
      try {
        const status = await api.getChainStatus();
        setChainStatus(status);
        const def = Math.max(0, status.epoch - 5);
        setCreatedAtEpoch(String(def));
        setCurrentEpochInput(String(status.epoch));
      } catch {
        // Keep defaults.
      }
      try {
        const addr = await keystore.getAddress();
        if (addr) {
          const objs = await api.getObjects(addr);
          setObjects(objs);
        }
      } catch {
        // No objects available — address mode still works.
      }
    })();
  }, []);

  const ladTypedObjects = useMemo(
    () => objects.filter((o) => o.isLadTyped === true),
    [objects],
  );

  const pickedObject = useMemo(
    () => ladTypedObjects.find((o) => o.id === pickedObjectId) ?? null,
    [ladTypedObjects, pickedObjectId],
  );

  // Pre-fill the mode whenever an object is picked or source changes.
  useEffect(() => {
    if (source === 'object' && pickedObject?.ladMode) {
      setMode(pickedObject.ladMode);
    }
  }, [source, pickedObject]);

  const handleSimulate = async () => {
    setError(null);
    setResult(null);
    setBusy(true);
    try {
      const v = parseInt(value, 10);
      const created = parseInt(createdAtEpoch, 10);
      const cur = parseInt(currentEpochInput, 10);
      const dw = mode === 'decaying' ? parseInt(decayWindow, 10) : undefined;

      if (!Number.isFinite(v) || v < 0) throw new Error('value must be ≥ 0');
      if (!Number.isFinite(created) || created < 0)
        throw new Error('created_at_epoch must be ≥ 0');
      if (!Number.isFinite(cur) || cur < 0)
        throw new Error('current_epoch must be ≥ 0');
      if (mode === 'decaying' && (!Number.isFinite(dw!) || dw! <= 0)) {
        throw new Error('decay_window must be > 0 for decaying mode');
      }

      const resp = await api.simulateLadVm({
        mode,
        action,
        value: v,
        created_at_epoch: created,
        current_epoch: cur,
        decay_window: dw,
      });
      setResult(resp);
    } catch (e: any) {
      setError(e?.message ?? 'Simulation failed');
    } finally {
      setBusy(false);
    }
  };

  const outcomeColor = !result
    ? '#9ca3af'
    : result.status === 'ok' &&
      (result.outcome === 'ok' || result.outcome === 'ticked-fresh')
    ? '#16a34a'
    : result.outcome.startsWith('ticked-')
    ? '#b45309'
    : '#dc2626';

  return (
    <SafeAreaView style={styles.container} edges={['bottom']}>
      <ScrollView contentContainerStyle={styles.scroll}>
        <View style={styles.headerCard}>
          <View style={styles.badge}>
            <Text style={styles.badgeText}>LAD-VM</Text>
          </View>
          <Text style={styles.headerTitle}>Resource preview</Text>
          <Text style={styles.headerBlurb}>
            Probe the substructural-resource lifecycle. Linear must be used once,
            affine can drop, decaying evaporates past its window.
          </Text>
        </View>

        {/* Source toggle */}
        <View style={styles.section}>
          <Text style={styles.label}>Source</Text>
          <View style={styles.toggleRow}>
            <TouchableOpacity
              style={[styles.toggle, source === 'address' && styles.toggleActive]}
              onPress={() => setSource('address')}
              activeOpacity={0.7}
            >
              <Text style={[styles.toggleText, source === 'address' && styles.toggleTextActive]}>
                Address
              </Text>
            </TouchableOpacity>
            <TouchableOpacity
              style={[styles.toggle, source === 'object' && styles.toggleActive]}
              onPress={() => setSource('object')}
              activeOpacity={0.7}
            >
              <Text style={[styles.toggleText, source === 'object' && styles.toggleTextActive]}>
                Object
              </Text>
            </TouchableOpacity>
          </View>
        </View>

        {/* Object picker */}
        {source === 'object' && (
          <View style={styles.section}>
            <Text style={styles.label}>Target Object</Text>
            <TouchableOpacity
              style={styles.picker}
              onPress={() => setPickerOpen(true)}
              activeOpacity={0.7}
              disabled={ladTypedObjects.length === 0}
            >
              <Text
                style={[
                  styles.pickerText,
                  !pickedObject && styles.pickerPlaceholder,
                ]}
                numberOfLines={1}
              >
                {pickedObject
                  ? `${pickedObject.name} · LAD/${pickedObject.ladMode}`
                  : ladTypedObjects.length === 0
                  ? 'No LAD-typed objects in your wallet'
                  : 'Select a LAD-typed object…'}
              </Text>
              <Text style={styles.pickerArrow}>{'›'}</Text>
            </TouchableOpacity>
            {pickedObject && (
              <Text style={styles.pickerOwner} numberOfLines={1}>
                Owner: {pickedObject.owner}
              </Text>
            )}
          </View>
        )}

        {/* Mode + Action */}
        <View style={styles.gridRow}>
          <View style={styles.gridCol}>
            <Text style={styles.label}>Mode</Text>
            <View style={styles.chipRow}>
              {MODES.map((m) => (
                <TouchableOpacity
                  key={m}
                  style={[styles.chipOption, mode === m && styles.chipOptionActive]}
                  onPress={() => setMode(m)}
                  activeOpacity={0.7}
                >
                  <Text
                    style={[
                      styles.chipOptionText,
                      mode === m && styles.chipOptionTextActive,
                    ]}
                  >
                    {m}
                  </Text>
                </TouchableOpacity>
              ))}
            </View>
          </View>
        </View>

        <View style={styles.gridRow}>
          <View style={styles.gridCol}>
            <Text style={styles.label}>Action</Text>
            <View style={styles.chipRow}>
              {ACTIONS.map((a) => (
                <TouchableOpacity
                  key={a}
                  style={[styles.chipOption, action === a && styles.chipOptionActive]}
                  onPress={() => setAction(a)}
                  activeOpacity={0.7}
                >
                  <Text
                    style={[
                      styles.chipOptionText,
                      action === a && styles.chipOptionTextActive,
                    ]}
                  >
                    {a}
                  </Text>
                </TouchableOpacity>
              ))}
            </View>
          </View>
        </View>

        {/* Numeric inputs */}
        <View style={styles.section}>
          <Text style={styles.label}>Value</Text>
          <TextInput
            style={styles.input}
            keyboardType="number-pad"
            value={value}
            onChangeText={setValue}
            placeholder="42"
            placeholderTextColor="#9ca3af"
          />
        </View>
        <View style={styles.section}>
          <Text style={styles.label}>Created at epoch</Text>
          <TextInput
            style={styles.input}
            keyboardType="number-pad"
            value={createdAtEpoch}
            onChangeText={setCreatedAtEpoch}
            placeholder="0"
            placeholderTextColor="#9ca3af"
          />
        </View>
        {mode === 'decaying' && (
          <View style={styles.section}>
            <Text style={styles.label}>Decay window</Text>
            <TextInput
              style={styles.input}
              keyboardType="number-pad"
              value={decayWindow}
              onChangeText={setDecayWindow}
              placeholder="10"
              placeholderTextColor="#9ca3af"
            />
          </View>
        )}
        <View style={styles.section}>
          <Text style={styles.label}>
            Current epoch{chainStatus ? ` (chain: ${chainStatus.epoch})` : ''}
          </Text>
          <TextInput
            style={styles.input}
            keyboardType="number-pad"
            value={currentEpochInput}
            onChangeText={setCurrentEpochInput}
            placeholder="0"
            placeholderTextColor="#9ca3af"
          />
        </View>

        {error && (
          <View style={styles.errorBanner}>
            <Text style={styles.errorText}>{error}</Text>
          </View>
        )}

        <TouchableOpacity
          style={styles.simulateBtn}
          onPress={handleSimulate}
          disabled={busy}
          activeOpacity={0.7}
        >
          {busy ? (
            <ActivityIndicator color="#ffffff" />
          ) : (
            <Text style={styles.simulateBtnText}>Simulate</Text>
          )}
        </TouchableOpacity>

        {result && (
          <View style={styles.resultCard}>
            <ResultRow
              label="Outcome"
              value={result.outcome}
              valueColor={outcomeColor}
            />
            <ResultRow
              label="Returned value"
              value={result.returned_value == null ? '—' : String(result.returned_value)}
            />
            <ResultRow
              label="Evaporated"
              value={result.is_evaporated_at_query ? 'yes' : 'no'}
              valueColor={result.is_evaporated_at_query ? '#dc2626' : '#16a34a'}
            />
            {!!result.detail && (
              <Text style={styles.resultDetail}>{result.detail}</Text>
            )}
          </View>
        )}
      </ScrollView>

      {/* Object picker modal */}
      <Modal visible={pickerOpen} animationType="slide" onRequestClose={() => setPickerOpen(false)}>
        <SafeAreaView style={styles.modalContainer} edges={['top', 'bottom']}>
          <View style={styles.modalHeader}>
            <Text style={styles.modalTitle}>Pick a LAD-typed object</Text>
            <TouchableOpacity onPress={() => setPickerOpen(false)} activeOpacity={0.6}>
              <Text style={styles.modalClose}>Close</Text>
            </TouchableOpacity>
          </View>
          <ScrollView contentContainerStyle={styles.modalScroll}>
            {ladTypedObjects.length === 0 ? (
              <Text style={styles.muted}>
                You have no LAD-typed objects. Create one to test the substructural
                resource lifecycle, or use Address mode for free-form simulation.
              </Text>
            ) : (
              ladTypedObjects.map((o) => (
                <TouchableOpacity
                  key={o.id}
                  style={[
                    styles.objectRow,
                    pickedObjectId === o.id && styles.objectRowActive,
                  ]}
                  onPress={() => {
                    setPickedObjectId(o.id);
                    setPickerOpen(false);
                  }}
                  activeOpacity={0.7}
                >
                  <View style={{ flex: 1 }}>
                    <Text style={styles.objectName}>{o.name}</Text>
                    <Text style={styles.objectId} numberOfLines={1}>
                      {o.id}
                    </Text>
                  </View>
                  <View style={styles.modeChip}>
                    <Text style={styles.modeChipText}>{o.ladMode}</Text>
                  </View>
                </TouchableOpacity>
              ))
            )}
          </ScrollView>
        </SafeAreaView>
      </Modal>
    </SafeAreaView>
  );
};

const ResultRow: React.FC<{ label: string; value: string; valueColor?: string }> = ({
  label,
  value,
  valueColor = '#374151',
}) => (
  <View style={styles.resultRow}>
    <Text style={styles.resultLabel}>{label}</Text>
    <Text style={[styles.resultValue, { color: valueColor }]}>{value}</Text>
  </View>
);

const styles = StyleSheet.create({
  container: { flex: 1, backgroundColor: '#f9fafb' },
  scroll: { padding: 16, paddingBottom: 40 },
  headerCard: {
    backgroundColor: '#f5f3ff',
    borderRadius: 12,
    borderWidth: 1,
    borderColor: '#c4b5fd',
    padding: 14,
    marginBottom: 16,
  },
  badge: {
    alignSelf: 'flex-start',
    paddingHorizontal: 8,
    paddingVertical: 2,
    borderRadius: 999,
    backgroundColor: '#ede9fe',
    borderWidth: 1,
    borderColor: '#c4b5fd',
    marginBottom: 6,
  },
  badgeText: { fontSize: 10, fontWeight: '700', color: '#7c3aed' },
  headerTitle: { fontSize: 14, fontWeight: '700', color: '#5b21b6' },
  headerBlurb: { fontSize: 12, color: '#6b21a8', marginTop: 4, lineHeight: 16 },
  section: { marginBottom: 14 },
  label: {
    fontSize: 11,
    fontWeight: '700',
    color: '#6b7280',
    textTransform: 'uppercase',
    letterSpacing: 0.5,
    marginBottom: 6,
  },
  toggleRow: { flexDirection: 'row', gap: 8 },
  toggle: {
    flex: 1,
    paddingVertical: 12,
    borderRadius: 10,
    backgroundColor: '#ffffff',
    borderWidth: 2,
    borderColor: '#e5e7eb',
    alignItems: 'center',
  },
  toggleActive: { borderColor: '#8b5cf6', backgroundColor: '#f5f3ff' },
  toggleText: { fontSize: 13, fontWeight: '600', color: '#6b7280' },
  toggleTextActive: { color: '#8b5cf6' },
  picker: {
    flexDirection: 'row',
    alignItems: 'center',
    backgroundColor: '#ffffff',
    borderRadius: 12,
    borderWidth: 1,
    borderColor: '#e5e7eb',
    paddingHorizontal: 14,
    paddingVertical: 14,
    minHeight: 48,
  },
  pickerText: { flex: 1, fontSize: 14, color: '#111827' },
  pickerPlaceholder: { color: '#9ca3af' },
  pickerArrow: { fontSize: 20, color: '#9ca3af', marginLeft: 8 },
  pickerOwner: {
    fontSize: 11,
    fontFamily: 'monospace',
    color: '#9ca3af',
    marginTop: 4,
  },
  gridRow: { marginBottom: 12 },
  gridCol: { flex: 1 },
  chipRow: { flexDirection: 'row', gap: 8 },
  chipOption: {
    flex: 1,
    paddingVertical: 10,
    borderRadius: 10,
    borderWidth: 1,
    borderColor: '#e5e7eb',
    backgroundColor: '#ffffff',
    alignItems: 'center',
  },
  chipOptionActive: { borderColor: '#8b5cf6', backgroundColor: '#f5f3ff' },
  chipOptionText: { fontSize: 12, fontWeight: '600', color: '#6b7280' },
  chipOptionTextActive: { color: '#8b5cf6' },
  input: {
    backgroundColor: '#ffffff',
    borderWidth: 1,
    borderColor: '#e5e7eb',
    borderRadius: 10,
    paddingHorizontal: 14,
    paddingVertical: 12,
    fontSize: 14,
    color: '#111827',
  },
  errorBanner: {
    backgroundColor: '#fef2f2',
    borderWidth: 1,
    borderColor: '#fca5a5',
    borderRadius: 10,
    padding: 12,
    marginBottom: 12,
  },
  errorText: { color: '#dc2626', fontSize: 12 },
  simulateBtn: {
    backgroundColor: '#8b5cf6',
    borderRadius: 12,
    paddingVertical: 14,
    alignItems: 'center',
    minHeight: 48,
    marginTop: 4,
  },
  simulateBtnText: { color: '#ffffff', fontSize: 15, fontWeight: '700' },
  resultCard: {
    marginTop: 16,
    backgroundColor: '#ffffff',
    borderRadius: 12,
    borderWidth: 1,
    borderColor: '#e5e7eb',
    padding: 14,
  },
  resultRow: {
    flexDirection: 'row',
    justifyContent: 'space-between',
    alignItems: 'center',
    paddingVertical: 6,
  },
  resultLabel: {
    fontSize: 11,
    fontWeight: '700',
    color: '#6b7280',
    textTransform: 'uppercase',
    letterSpacing: 0.4,
  },
  resultValue: { fontSize: 13, fontWeight: '600' },
  resultDetail: {
    fontSize: 11,
    color: '#6b7280',
    marginTop: 8,
    paddingTop: 8,
    borderTopWidth: 1,
    borderTopColor: '#f3f4f6',
    lineHeight: 16,
  },
  modalContainer: { flex: 1, backgroundColor: '#f9fafb' },
  modalHeader: {
    flexDirection: 'row',
    justifyContent: 'space-between',
    alignItems: 'center',
    paddingHorizontal: 16,
    paddingVertical: 12,
    borderBottomWidth: 1,
    borderBottomColor: '#e5e7eb',
    backgroundColor: '#ffffff',
  },
  modalTitle: { fontSize: 16, fontWeight: '700', color: '#111827' },
  modalClose: { fontSize: 14, fontWeight: '600', color: '#06b6d4' },
  modalScroll: { padding: 16, paddingBottom: 32 },
  muted: { fontSize: 13, color: '#9ca3af', textAlign: 'center', paddingVertical: 32 },
  objectRow: {
    flexDirection: 'row',
    alignItems: 'center',
    backgroundColor: '#ffffff',
    borderRadius: 12,
    borderWidth: 1,
    borderColor: '#e5e7eb',
    padding: 14,
    marginBottom: 8,
    gap: 12,
  },
  objectRowActive: { borderColor: '#8b5cf6', backgroundColor: '#f5f3ff' },
  objectName: { fontSize: 14, fontWeight: '600', color: '#111827' },
  objectId: {
    fontSize: 11,
    fontFamily: 'monospace',
    color: '#9ca3af',
    marginTop: 2,
  },
  modeChip: {
    paddingHorizontal: 10,
    paddingVertical: 4,
    borderRadius: 999,
    backgroundColor: '#ede9fe',
    borderWidth: 1,
    borderColor: '#c4b5fd',
  },
  modeChipText: { fontSize: 11, fontWeight: '700', color: '#7c3aed' },
});

export default LadVmScreen;
