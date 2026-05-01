/**
 * DemurrageScreen — Idle-balance fee on the EvaporChain substrate.
 *
 * Mirrors extension/src/components/DemurrageBadge.tsx data flow but
 * promoted to a full screen for mobile.
 *
 *   POST /api/demurrage/owed       — pure compute (no mutation)
 *   POST /api/tx/settle_demurrage  — debits owed, credits refresh-pool
 *
 * Settle requires an ML-DSA signature over the canonical payload:
 *   JSON({type:"settle_demurrage",from,current_epoch})
 * — exactly the shape api.rs §post_settle_demurrage L5499 builds for
 * verification. We sign with `keystore.signMessage` which calls
 * `ml_dsa65.sign(sk, message)` directly.
 *
 * The UI hides the settle action when `is_disabled` is true or
 * `last_touched_epoch == 0` (newly-funded account, nothing to charge).
 */

import React, { useCallback, useEffect, useState } from 'react';
import {
  View,
  Text,
  TouchableOpacity,
  ScrollView,
  StyleSheet,
  ActivityIndicator,
  Alert,
  RefreshControl,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import * as LocalAuthentication from 'expo-local-authentication';
import { api } from '../utils/api';
import type {
  AccountDetail,
  ChainStatus,
  DemurrageOwedResp,
} from '../utils/api';
import { keystore } from '../utils/keystore';

// Genesis demurrage params — must match api.rs §post_settle_demurrage:
// `DemurrageParams::new(1, 1024)`. Same numbers used in extension.
const LAMBDA_BASE_PPM = 1;
const THRESHOLD = 1024;

const DemurrageScreen: React.FC = () => {
  const [account, setAccount] = useState<AccountDetail | null>(null);
  const [chain, setChain] = useState<ChainStatus | null>(null);
  const [owed, setOwed] = useState<DemurrageOwedResp | null>(null);
  const [loading, setLoading] = useState(true);
  const [settling, setSettling] = useState(false);

  const load = useCallback(async () => {
    try {
      const address = await keystore.getAddress();
      if (!address) return;

      const [acctR, chainR] = await Promise.allSettled([
        api.getAccountDetail(address),
        api.getChainStatus(),
      ]);

      const acct = acctR.status === 'fulfilled' ? acctR.value : null;
      const ch = chainR.status === 'fulfilled' ? chainR.value : null;
      setAccount(acct);
      setChain(ch);

      if (acct && ch) {
        try {
          const owedResp = await api.getDemurrageOwed({
            balance: acct.balance,
            last_touched_epoch: acct.last_touched_epoch ?? 0,
            current_epoch: ch.epoch,
            lambda_base_ppm: LAMBDA_BASE_PPM,
            threshold: THRESHOLD,
          });
          setOwed(owedResp);
        } catch {
          /* leave previous owed */
        }
      }
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const handleSettle = async () => {
    if (!account || !chain || !owed) return;
    if (owed.owed <= 0) {
      Alert.alert('Nothing owed', 'No demurrage to settle at this epoch.');
      return;
    }

    const auth = await LocalAuthentication.authenticateAsync({
      promptMessage: 'Confirm demurrage settlement',
      cancelLabel: 'Cancel',
    });
    if (!auth.success) return;

    setSettling(true);
    try {
      const from = account.address;
      const currentEpoch = chain.epoch;

      // Canonical payload — must match api.rs §post_settle_demurrage L5499
      // BYTE-FOR-BYTE. The Rust handler builds the exact same string with
      // `format!("{{\"type\":\"settle_demurrage\",\"from\":\"{}\",\"current_epoch\":{}}}", from, current_epoch)`
      // so we cannot use `JSON.stringify({...})` (object-key order matters).
      const canonical = `{"type":"settle_demurrage","from":"${from}","current_epoch":${currentEpoch}}`;

      const sig = await keystore.signMessage(new TextEncoder().encode(canonical));
      const pk = await keystore.getPublicKey();
      if (!pk) {
        Alert.alert('Error', 'No public key on device.');
        return;
      }

      const resp = await api.settleDemurrage(from, sig, pk);
      if (resp.status === 'settled') {
        Alert.alert(
          'Settled',
          `${resp.settled.toLocaleString()} EVAP debited. New balance: ${resp.new_balance.toLocaleString()}`,
        );
        await load();
      } else if (resp.status === 'nothing_owed') {
        Alert.alert('Nothing owed', resp.detail);
      } else {
        Alert.alert('Failed', resp.detail || 'Settle rejected');
      }
    } catch (e) {
      Alert.alert('Error', String(e));
    } finally {
      setSettling(false);
    }
  };

  if (loading) {
    return (
      <View style={styles.centered}>
        <ActivityIndicator size="large" color="#06b6d4" />
      </View>
    );
  }

  const hasTouch = (account?.last_touched_epoch ?? 0) > 0;
  const owedAmount = owed?.owed ?? 0;
  const elapsed = (chain?.epoch ?? 0) - (account?.last_touched_epoch ?? 0);
  const safeElapsed = Math.max(0, elapsed);
  const isDisabled = owed?.is_disabled === true;

  return (
    <SafeAreaView style={styles.container} edges={['bottom']}>
      <ScrollView
        contentContainerStyle={styles.scroll}
        refreshControl={<RefreshControl refreshing={false} onRefresh={load} tintColor="#06b6d4" />}
      >
        {/* Headline */}
        <View
          style={[
            styles.card,
            owedAmount > 0 ? styles.cardWarn : null,
          ]}
        >
          <Text style={styles.label}>Demurrage owed</Text>
          <View style={styles.bigRow}>
            <Text style={[styles.bigVal, owedAmount > 0 ? { color: '#f59e0b' } : null]}>
              {owedAmount.toLocaleString()}
            </Text>
            <Text style={styles.bigUnit}>EVAP</Text>
          </View>
          {!hasTouch ? (
            <Text style={styles.subtle}>
              No outbound activity yet — demurrage anchor is unset.
            </Text>
          ) : isDisabled ? (
            <Text style={styles.subtle}>Demurrage disabled at chain config.</Text>
          ) : owedAmount === 0 ? (
            <Text style={styles.subtle}>Nothing to settle this epoch.</Text>
          ) : null}
        </View>

        {/* Breakdown */}
        <View style={styles.card}>
          <Text style={styles.label}>Breakdown</Text>
          <Row k="Balance" v={`${(account?.balance ?? 0).toLocaleString()} EVAP`} />
          <Row k="After settle" v={`${Math.max(0, (account?.balance ?? 0) - owedAmount).toLocaleString()} EVAP`} />
          <Row k="Elapsed epochs" v={safeElapsed.toLocaleString()} />
          <Row k="Since epoch" v={String(account?.last_touched_epoch ?? 0)} />
          <Row k="Current epoch" v={String(chain?.epoch ?? 0)} />
          <Row
            k="Rate (per epoch)"
            v={owed ? `${owed.rate_ppm.toLocaleString()} ppm` : '—'}
          />
          <Row k="λ_base" v={`${LAMBDA_BASE_PPM} ppm`} />
          <Row k="Threshold" v={THRESHOLD.toLocaleString()} />
        </View>

        {/* Action */}
        <TouchableOpacity
          style={[
            styles.settleBtn,
            (settling || owedAmount <= 0 || !hasTouch) && styles.settleBtnDisabled,
          ]}
          disabled={settling || owedAmount <= 0 || !hasTouch}
          onPress={handleSettle}
          activeOpacity={0.7}
        >
          {settling ? (
            <ActivityIndicator color="#ffffff" />
          ) : (
            <Text style={styles.settleBtnText}>
              {owedAmount > 0 ? `Settle ${owedAmount.toLocaleString()} EVAP` : 'Nothing to settle'}
            </Text>
          )}
        </TouchableOpacity>

        <View style={styles.note}>
          <Text style={styles.noteText}>
            Settling debits the owed amount from your balance and credits the protocol-owned
            refresh pool under namespace DEMU. The owed amount is recomputed every block from
            last_touched_epoch and the piecewise-log rate.
          </Text>
        </View>
      </ScrollView>
    </SafeAreaView>
  );
};

const Row: React.FC<{ k: string; v: string }> = ({ k, v }) => (
  <View style={styles.kvRow}>
    <Text style={styles.kvKey}>{k}</Text>
    <Text style={styles.kvVal}>{v}</Text>
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
  cardWarn: { backgroundColor: '#fffbeb', borderColor: '#fde68a' },
  label: {
    fontSize: 11,
    color: '#6b7280',
    fontWeight: '600',
    textTransform: 'uppercase',
    letterSpacing: 0.5,
    marginBottom: 6,
  },
  bigRow: { flexDirection: 'row', alignItems: 'baseline', gap: 6 },
  bigVal: { fontSize: 28, fontWeight: '700', color: '#111827' },
  bigUnit: { fontSize: 12, color: '#9ca3af' },
  subtle: { fontSize: 12, color: '#6b7280', marginTop: 6 },
  kvRow: { flexDirection: 'row', justifyContent: 'space-between', paddingVertical: 6 },
  kvKey: { fontSize: 12, color: '#6b7280' },
  kvVal: { fontSize: 13, color: '#111827', fontWeight: '600' },
  settleBtn: {
    backgroundColor: '#06b6d4',
    borderRadius: 12,
    paddingVertical: 14,
    alignItems: 'center',
    marginBottom: 12,
    minHeight: 48,
    justifyContent: 'center',
  },
  settleBtnDisabled: { opacity: 0.5 },
  settleBtnText: { color: '#ffffff', fontSize: 15, fontWeight: '700' },
  note: {
    backgroundColor: '#ecfeff',
    borderColor: '#a5f3fc',
    borderWidth: 1,
    borderRadius: 10,
    padding: 12,
  },
  noteText: { fontSize: 11, color: '#0e7490', lineHeight: 16 },
});

export default DemurrageScreen;
