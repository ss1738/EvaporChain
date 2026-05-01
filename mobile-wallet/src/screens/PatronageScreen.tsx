/**
 * PatronageScreen — Pre-fund eviction immunity for objects.
 *
 * Mirrors extension/src/components/PatronageScreen.tsx data flow:
 *  - GET /api/patronage/status      — pool stats + canonical ns hex
 *  - GET /api/patronage/immune      — per-object immunity probe
 *  - POST /api/patronage/pledge     — open a covenant
 *  - POST /api/patronage/honour     — pay one epoch's donation
 *  - POST /api/patronage/revoke     — close + refund unused balance
 *
 * Fields match api.rs §PatronagePledgeReq / §PatronageStatusResp.
 * Fetch-once-on-mount; manual refresh via header button.
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
import type {
  ChainObject,
  ChainStatus,
  PatronageStatusResp,
  PatronageImmuneResp,
} from '../utils/api';
import { keystore } from '../utils/keystore';

const stripHex = (s: string): string => (s.startsWith('0x') ? s.slice(2) : s);

const PatronageScreen: React.FC = () => {
  const [pool, setPool] = useState<PatronageStatusResp | null>(null);
  const [chain, setChain] = useState<ChainStatus | null>(null);
  const [objects, setObjects] = useState<ChainObject[]>([]);
  const [immunities, setImmunities] = useState<Record<string, PatronageImmuneResp>>({});
  const [openId, setOpenId] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [loading, setLoading] = useState(true);

  const currentEpoch = chain?.epoch ?? 0;

  const loadAll = useCallback(async () => {
    try {
      const address = await keystore.getAddress();
      if (!address) return;

      const [statusR, chainR, objsR] = await Promise.allSettled([
        api.getPatronageStatus(),
        api.getChainStatus(),
        api.getObjects(address),
      ]);

      if (statusR.status === 'fulfilled') setPool(statusR.value);
      if (chainR.status === 'fulfilled') setChain(chainR.value);
      const objs = objsR.status === 'fulfilled' ? objsR.value : [];
      setObjects(objs);

      const epoch = chainR.status === 'fulfilled' ? chainR.value.epoch : 0;
      const map: Record<string, PatronageImmuneResp> = {};
      for (const obj of objs) {
        try {
          const idHex = stripHex(obj.id);
          const im = await api.getPatronageImmunity(idHex, epoch);
          map[idHex] = im;
        } catch {
          /* skip */
        }
      }
      setImmunities(map);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadAll();
  }, [loadAll]);

  const handlePledge = async (
    objectIdHex: string,
    namespaceHex: string,
    donationPerEpoch: number,
    epochs: number,
  ) => {
    setBusy(true);
    try {
      const resp = await api.pledgePatronage({
        object_id_hex: objectIdHex,
        namespace_id_hex: namespaceHex,
        donation_per_epoch: donationPerEpoch,
        epochs,
        current_epoch: currentEpoch,
      });
      if (resp.status === 'pledged') {
        Alert.alert('Pledged', `${resp.pre_funded.toLocaleString()} EVAP pre-funded.`);
        setOpenId(null);
        await loadAll();
      } else {
        Alert.alert('Failed', resp.detail || 'Pledge rejected');
      }
    } catch (e) {
      Alert.alert('Error', String(e));
    } finally {
      setBusy(false);
    }
  };

  const handleHonour = async (idHex: string) => {
    setBusy(true);
    try {
      const r = await api.honourPatronage({ object_id_hex: idHex, epoch: currentEpoch });
      if (r.status === 'honoured') {
        Alert.alert('Honoured', `Donated ${r.donated.toLocaleString()} EVAP — score ${r.patronage_score}`);
        await loadAll();
      } else {
        Alert.alert('Failed', r.detail || 'Honour rejected');
      }
    } catch (e) {
      Alert.alert('Error', String(e));
    } finally {
      setBusy(false);
    }
  };

  const handleRevoke = async (idHex: string) => {
    setBusy(true);
    try {
      const r = await api.revokePatronage({ object_id_hex: idHex, epoch: currentEpoch });
      if (r.status === 'revoked') {
        Alert.alert('Revoked', `Refunded ${(r.refunded ?? 0).toLocaleString()} EVAP`);
        await loadAll();
      } else {
        Alert.alert('Failed', r.detail || 'Revoke rejected');
      }
    } catch (e) {
      Alert.alert('Error', String(e));
    } finally {
      setBusy(false);
    }
  };

  if (loading) {
    return (
      <View style={styles.centered}>
        <ActivityIndicator size="large" color="#06b6d4" />
      </View>
    );
  }

  return (
    <SafeAreaView style={styles.container} edges={['bottom']}>
      <ScrollView
        contentContainerStyle={styles.scroll}
        refreshControl={<RefreshControl refreshing={false} onRefresh={loadAll} tintColor="#06b6d4" />}
      >
        {/* Pool stats */}
        <View style={styles.card}>
          <Text style={styles.cardLabel}>Patronage Pool — Epoch {currentEpoch}</Text>
          {pool ? (
            <View style={styles.statRow}>
              <Stat label="Active" value={String(pool.active_covenants)} />
              <Stat label="Pre-funded" value={pool.total_pre_funded.toLocaleString()} sub="EVAP" />
              <Stat label="Score" value={pool.total_active_score.toLocaleString()} />
            </View>
          ) : (
            <Text style={styles.muted}>Loading pool…</Text>
          )}
          {pool?.patronage_ns_hex ? (
            <Text style={styles.nsText} numberOfLines={1}>
              ns: {pool.patronage_ns_hex.slice(0, 32)}…
            </Text>
          ) : null}
        </View>

        {/* Object list */}
        <Text style={styles.sectionLabel}>Your objects</Text>
        {objects.length === 0 ? (
          <View style={styles.emptyCard}>
            <Text style={styles.muted}>No objects to patronise.</Text>
          </View>
        ) : (
          objects.map((obj) => {
            const idHex = stripHex(obj.id);
            const im = immunities[idHex];
            const open = openId === obj.id;
            const isImmune = im?.immune === true;
            return (
              <PatronageRow
                key={obj.id}
                obj={obj}
                idHex={idHex}
                isImmune={isImmune}
                immunityScore={im?.patronage_score ?? 0}
                isOpen={open}
                defaultNs={pool?.patronage_ns_hex ?? ''}
                currentEpoch={currentEpoch}
                busy={busy}
                onToggle={() => setOpenId(open ? null : obj.id)}
                onPledge={(ns, dpe, ep) => handlePledge(idHex, ns, dpe, ep)}
                onHonour={() => handleHonour(idHex)}
                onRevoke={() => handleRevoke(idHex)}
              />
            );
          })
        )}
      </ScrollView>
    </SafeAreaView>
  );
};

const Stat: React.FC<{ label: string; value: string; sub?: string }> = ({ label, value, sub }) => (
  <View style={styles.statItem}>
    <Text style={styles.statLabel}>{label}</Text>
    <View style={styles.statValueRow}>
      <Text style={styles.statValue}>{value}</Text>
      {sub ? <Text style={styles.statSub}>{sub}</Text> : null}
    </View>
  </View>
);

interface RowProps {
  obj: ChainObject;
  idHex: string;
  isImmune: boolean;
  immunityScore: number;
  isOpen: boolean;
  defaultNs: string;
  currentEpoch: number;
  busy: boolean;
  onToggle: () => void;
  onPledge: (namespaceHex: string, donationPerEpoch: number, epochs: number) => void;
  onHonour: () => void;
  onRevoke: () => void;
}

const PatronageRow: React.FC<RowProps> = ({
  obj,
  idHex,
  isImmune,
  immunityScore,
  isOpen,
  defaultNs,
  currentEpoch,
  busy,
  onToggle,
  onPledge,
  onHonour,
  onRevoke,
}) => {
  const [donationPerEpoch, setDonationPerEpoch] = useState('100');
  const [epochs, setEpochs] = useState('10');
  const [namespace, setNamespace] = useState<string>(defaultNs);

  useEffect(() => {
    if (!namespace && defaultNs) setNamespace(defaultNs);
  }, [defaultNs, namespace]);

  const total = (parseInt(donationPerEpoch, 10) || 0) * (parseInt(epochs, 10) || 0);

  return (
    <View style={styles.objCard}>
      <View style={styles.objHeader}>
        <View style={{ flex: 1 }}>
          <Text style={styles.objName} numberOfLines={1}>
            {obj.name || obj.id.slice(0, 12)}
          </Text>
          <Text style={styles.objId} numberOfLines={1}>
            {obj.id}
          </Text>
        </View>
        {isImmune ? (
          <View style={[styles.pill, styles.pillEmerald]}>
            <Text style={[styles.pillText, { color: '#22c55e' }]}>Immune · {immunityScore}</Text>
          </View>
        ) : (
          <View style={styles.pill}>
            <Text style={[styles.pillText, { color: '#6b7280' }]}>No covenant</Text>
          </View>
        )}
      </View>

      <View style={styles.btnRow}>
        <TouchableOpacity
          disabled={busy}
          style={[styles.btn, styles.btnCyan]}
          onPress={onToggle}
          activeOpacity={0.7}
        >
          <Text style={styles.btnCyanText}>{isOpen ? 'Close' : isImmune ? 'Renew' : 'Pledge'}</Text>
        </TouchableOpacity>
        {isImmune ? (
          <>
            <TouchableOpacity
              disabled={busy}
              style={[styles.btn, styles.btnPurple]}
              onPress={onHonour}
              activeOpacity={0.7}
            >
              <Text style={styles.btnPurpleText}>Honour</Text>
            </TouchableOpacity>
            <TouchableOpacity
              disabled={busy}
              style={[styles.btn, styles.btnRed]}
              onPress={onRevoke}
              activeOpacity={0.7}
            >
              <Text style={styles.btnRedText}>Revoke</Text>
            </TouchableOpacity>
          </>
        ) : null}
      </View>

      {isOpen ? (
        <View style={styles.formSection}>
          <Field label="Donation per epoch (EVAP)">
            <TextInput
              style={styles.input}
              value={donationPerEpoch}
              onChangeText={setDonationPerEpoch}
              keyboardType="numeric"
              placeholder="100"
              placeholderTextColor="#9ca3af"
            />
          </Field>
          <Field label="Epochs">
            <TextInput
              style={styles.input}
              value={epochs}
              onChangeText={setEpochs}
              keyboardType="numeric"
              placeholder="10"
              placeholderTextColor="#9ca3af"
            />
          </Field>
          <Field label="Current epoch (auto)">
            <TextInput
              style={[styles.input, styles.inputDisabled]}
              value={String(currentEpoch)}
              editable={false}
            />
          </Field>
          <Field label="Namespace (hex)">
            <TextInput
              style={[styles.input, styles.inputMono]}
              value={namespace}
              onChangeText={(t) => setNamespace(t.replace(/^0x/, ''))}
              autoCapitalize="none"
              placeholder={defaultNs || 'namespace_id_hex'}
              placeholderTextColor="#9ca3af"
            />
          </Field>
          <View style={styles.confirmRow}>
            <Text style={styles.muted}>
              Total pre-fund: <Text style={styles.bold}>{total.toLocaleString()}</Text> EVAP
            </Text>
            <TouchableOpacity
              disabled={busy || !namespace || total <= 0}
              style={[styles.btn, styles.btnConfirm, (busy || !namespace || total <= 0) && { opacity: 0.5 }]}
              onPress={() => {
                const dpe = parseInt(donationPerEpoch, 10);
                const ep = parseInt(epochs, 10);
                if (!Number.isFinite(dpe) || dpe <= 0) return;
                if (!Number.isFinite(ep) || ep <= 0) return;
                onPledge(namespace, dpe, ep);
              }}
              activeOpacity={0.7}
            >
              <Text style={styles.btnConfirmText}>{busy ? '…' : 'Confirm'}</Text>
            </TouchableOpacity>
          </View>
          <Text style={styles.objId}>Object: {idHex.slice(0, 24)}…</Text>
        </View>
      ) : null}
    </View>
  );
};

const Field: React.FC<{ label: string; children: React.ReactNode }> = ({ label, children }) => (
  <View style={{ marginBottom: 8 }}>
    <Text style={styles.fieldLabel}>{label}</Text>
    {children}
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
  cardLabel: {
    fontSize: 11,
    color: '#6b7280',
    fontWeight: '600',
    marginBottom: 10,
    textTransform: 'uppercase',
    letterSpacing: 0.5,
  },
  statRow: { flexDirection: 'row', justifyContent: 'space-between' },
  statItem: { flex: 1 },
  statLabel: { fontSize: 10, color: '#9ca3af', textTransform: 'uppercase', letterSpacing: 0.4 },
  statValueRow: { flexDirection: 'row', alignItems: 'baseline', gap: 4, marginTop: 2 },
  statValue: { fontSize: 16, fontWeight: '700', color: '#111827' },
  statSub: { fontSize: 10, color: '#9ca3af' },
  nsText: { fontFamily: 'monospace', fontSize: 10, color: '#9ca3af', marginTop: 8 },
  muted: { fontSize: 12, color: '#9ca3af' },
  bold: { fontWeight: '700', color: '#111827' },
  sectionLabel: {
    fontSize: 11,
    color: '#6b7280',
    fontWeight: '600',
    marginBottom: 8,
    marginTop: 8,
    textTransform: 'uppercase',
    letterSpacing: 0.5,
  },
  emptyCard: {
    backgroundColor: '#ffffff',
    borderRadius: 12,
    padding: 24,
    alignItems: 'center',
    borderWidth: 1,
    borderColor: '#e5e7eb',
  },
  objCard: {
    backgroundColor: '#ffffff',
    borderRadius: 12,
    padding: 12,
    marginBottom: 8,
    borderWidth: 1,
    borderColor: '#e5e7eb',
  },
  objHeader: { flexDirection: 'row', alignItems: 'center', gap: 8 },
  objName: { fontSize: 14, fontWeight: '600', color: '#111827' },
  objId: { fontSize: 10, fontFamily: 'monospace', color: '#9ca3af', marginTop: 2 },
  pill: {
    paddingHorizontal: 8,
    paddingVertical: 2,
    borderRadius: 999,
    backgroundColor: '#f3f4f6',
    borderWidth: 1,
    borderColor: '#e5e7eb',
  },
  pillEmerald: { backgroundColor: '#ecfdf5', borderColor: '#a7f3d0' },
  pillText: { fontSize: 10, fontWeight: '600' },
  btnRow: { flexDirection: 'row', gap: 6, marginTop: 10 },
  btn: {
    flex: 1,
    paddingVertical: 8,
    borderRadius: 8,
    alignItems: 'center',
    borderWidth: 1,
  },
  btnCyan: { backgroundColor: '#ecfeff', borderColor: '#67e8f9' },
  btnCyanText: { color: '#06b6d4', fontSize: 12, fontWeight: '600' },
  btnPurple: { backgroundColor: '#f5f3ff', borderColor: '#c4b5fd' },
  btnPurpleText: { color: '#8b5cf6', fontSize: 12, fontWeight: '600' },
  btnRed: { backgroundColor: '#fef2f2', borderColor: '#fecaca' },
  btnRedText: { color: '#ef4444', fontSize: 12, fontWeight: '600' },
  formSection: {
    marginTop: 12,
    paddingTop: 12,
    borderTopWidth: 1,
    borderTopColor: '#f3f4f6',
  },
  fieldLabel: {
    fontSize: 10,
    color: '#9ca3af',
    fontWeight: '600',
    marginBottom: 4,
    textTransform: 'uppercase',
    letterSpacing: 0.4,
  },
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
  inputDisabled: { color: '#9ca3af', backgroundColor: '#f3f4f6' },
  inputMono: { fontFamily: 'monospace', fontSize: 11 },
  confirmRow: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
    marginTop: 6,
    marginBottom: 6,
  },
  btnConfirm: { flex: 0, paddingHorizontal: 16, backgroundColor: '#06b6d4', borderColor: '#06b6d4' },
  btnConfirmText: { color: '#ffffff', fontSize: 12, fontWeight: '700' },
});

export default PatronageScreen;
