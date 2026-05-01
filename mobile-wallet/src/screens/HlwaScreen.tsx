/**
 * HlwaScreen — Half-Life Wrapped Asset effective_supply decay.
 *
 * Mirrors extension/src/components/HlwaCard.tsx.
 *
 *   POST /api/hlwa/effective_supply  — pure compute
 *   POST /api/hlwa/re_attest         — pure simulation (no broadcast)
 *
 * Defaults from api.rs doc example (1M / λ=500 / ep=0).  Re-attestation
 * is read-only — it returns the post-attestation state without mutating
 * chain state. No signature required.
 */

import React, { useCallback, useEffect, useState } from 'react';
import {
  View,
  Text,
  TouchableOpacity,
  ScrollView,
  StyleSheet,
  ActivityIndicator,
  RefreshControl,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { api } from '../utils/api';
import type { ChainStatus, HlwaSupplyResp } from '../utils/api';

const LAMBDA_EPOCHS = 500;

const HlwaScreen: React.FC = () => {
  const [chain, setChain] = useState<ChainStatus | null>(null);
  const [currentSupply] = useState(1_000_000);
  const [originAttested, setOriginAttested] = useState(1_000_000);
  const [lastAttestedEpoch, setLastAttestedEpoch] = useState(0);
  const [supply, setSupply] = useState<HlwaSupplyResp | null>(null);
  const [loading, setLoading] = useState(true);
  const [reAttesting, setReAttesting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const currentEpoch = chain?.epoch ?? 100;

  const fetchSupply = useCallback(async () => {
    setError(null);
    try {
      const resp = await api.getHlwaEffectiveSupply({
        current_supply: currentSupply,
        origin_attested_supply: originAttested,
        last_attested_epoch: lastAttestedEpoch,
        attestation_lambda_epochs: LAMBDA_EPOCHS,
        current_epoch: currentEpoch,
      });
      setSupply(resp);
      if (resp.status !== 'ok') setError(resp.detail || 'supply lookup failed');
    } catch (e) {
      setError(String(e));
    }
  }, [currentSupply, originAttested, lastAttestedEpoch, currentEpoch]);

  const load = useCallback(async () => {
    try {
      const c = await api.getChainStatus();
      setChain(c);
      await fetchSupply();
    } catch {
      /* ignore */
    } finally {
      setLoading(false);
    }
  }, [fetchSupply]);

  useEffect(() => {
    load();
  }, [load]);

  // Re-fetch supply whenever its inputs change.
  useEffect(() => {
    if (!loading) fetchSupply();
  }, [fetchSupply, loading]);

  const handleReAttest = async () => {
    setReAttesting(true);
    setError(null);
    try {
      const resp = await api.reAttestHlwa({
        current_supply: currentSupply,
        origin_attested_supply: originAttested,
        last_attested_epoch: lastAttestedEpoch,
        attestation_lambda_epochs: LAMBDA_EPOCHS,
        current_epoch: currentEpoch,
      });
      if (resp.status === 'ok') {
        setOriginAttested(resp.new_attested_supply);
        setLastAttestedEpoch(resp.new_last_attested_epoch);
      } else {
        setError(resp.detail || 're-attest failed');
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setReAttesting(false);
    }
  };

  if (loading) {
    return (
      <View style={styles.centered}>
        <ActivityIndicator size="large" color="#06b6d4" />
      </View>
    );
  }

  const ratio = supply && supply.current_supply > 0 ? supply.effective_supply / supply.current_supply : 0;
  const ratioPct = Math.max(0, Math.min(100, ratio * 100));
  const excess = supply?.excess_to_burn ?? 0;
  const ratioColor =
    ratioPct > 80 ? '#22c55e' : ratioPct > 50 ? '#f59e0b' : '#ef4444';

  return (
    <SafeAreaView style={styles.container} edges={['bottom']}>
      <ScrollView
        contentContainerStyle={styles.scroll}
        refreshControl={<RefreshControl refreshing={false} onRefresh={load} tintColor="#06b6d4" />}
      >
        <View style={styles.card}>
          <View style={styles.row}>
            <View style={{ flex: 1 }}>
              <Text style={styles.title}>HLWA wrapped asset</Text>
              <Text style={styles.subtle}>
                Effective supply decays as attestation freshness ages (λ-decay).
              </Text>
            </View>
            <View style={styles.lambdaPill}>
              <Text style={styles.lambdaPillText}>λ {LAMBDA_EPOCHS} ep</Text>
            </View>
          </View>

          {error ? (
            <View style={styles.errBox}>
              <Text style={styles.errText}>{error}</Text>
            </View>
          ) : null}

          <View style={styles.statGrid}>
            <Stat label="Current supply" value={currentSupply.toLocaleString()} unit="HLWA" />
            <Stat
              label="Effective"
              value={supply ? supply.effective_supply.toLocaleString() : '—'}
              unit="HLWA"
              highlight
            />
            <Stat label="Last attested" value={`Epoch ${lastAttestedEpoch.toLocaleString()}`} />
            <Stat label="Current" value={`Epoch ${currentEpoch.toLocaleString()}`} />
          </View>

          <View style={{ marginTop: 12 }}>
            <View style={styles.barLabelRow}>
              <Text style={styles.barLabel}>Effective / current</Text>
              <Text style={styles.barLabel}>{(ratio * 100).toFixed(1)}%</Text>
            </View>
            <View style={styles.barBg}>
              <View
                style={[styles.barFill, { width: `${ratioPct}%`, backgroundColor: ratioColor }]}
              />
            </View>
          </View>

          {excess > 0 ? (
            <View style={styles.warnBox}>
              <Text style={styles.warnText}>
                <Text style={{ fontWeight: '700' }}>{excess.toLocaleString()} HLWA</Text> excess —
                bridge would burn this on the next anti-inflation gate.
              </Text>
            </View>
          ) : null}

          <View style={styles.btnRow}>
            <TouchableOpacity
              style={[styles.btn, styles.btnGhost]}
              disabled={reAttesting}
              onPress={fetchSupply}
              activeOpacity={0.7}
            >
              <Text style={styles.btnGhostText}>Refresh</Text>
            </TouchableOpacity>
            <TouchableOpacity
              style={[styles.btn, styles.btnCyan]}
              disabled={reAttesting}
              onPress={handleReAttest}
              activeOpacity={0.7}
            >
              {reAttesting ? (
                <ActivityIndicator color="#ffffff" />
              ) : (
                <Text style={styles.btnCyanText}>Re-attest from origin</Text>
              )}
            </TouchableOpacity>
          </View>

          <Text style={styles.note}>
            Re-attestation is simulated read-only — the endpoint returns post-attestation state
            without mutating chain state.
          </Text>
        </View>
      </ScrollView>
    </SafeAreaView>
  );
};

const Stat: React.FC<{ label: string; value: string; unit?: string; highlight?: boolean }> = ({
  label,
  value,
  unit,
  highlight,
}) => (
  <View style={styles.stat}>
    <Text style={styles.statLabel}>{label}</Text>
    <View style={styles.statValRow}>
      <Text style={[styles.statVal, highlight && { color: '#06b6d4' }]}>{value}</Text>
      {unit ? <Text style={styles.statUnit}>{unit}</Text> : null}
    </View>
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
  row: { flexDirection: 'row', alignItems: 'flex-start' },
  title: { fontSize: 14, fontWeight: '700', color: '#111827' },
  subtle: { fontSize: 11, color: '#6b7280', marginTop: 2 },
  lambdaPill: {
    backgroundColor: '#ecfeff',
    borderColor: '#a5f3fc',
    borderWidth: 1,
    borderRadius: 999,
    paddingHorizontal: 8,
    paddingVertical: 2,
  },
  lambdaPillText: { color: '#0e7490', fontSize: 10, fontWeight: '600' },
  errBox: {
    backgroundColor: '#fef2f2',
    borderColor: '#fecaca',
    borderWidth: 1,
    borderRadius: 8,
    padding: 8,
    marginTop: 8,
  },
  errText: { color: '#ef4444', fontSize: 11 },
  statGrid: {
    marginTop: 12,
    flexDirection: 'row',
    flexWrap: 'wrap',
  },
  stat: { width: '50%', marginBottom: 10 },
  statLabel: { fontSize: 10, color: '#9ca3af', textTransform: 'uppercase', letterSpacing: 0.4 },
  statValRow: { flexDirection: 'row', alignItems: 'baseline', gap: 4, marginTop: 2 },
  statVal: { fontSize: 14, fontWeight: '700', color: '#111827' },
  statUnit: { fontSize: 10, color: '#9ca3af' },
  barLabelRow: { flexDirection: 'row', justifyContent: 'space-between', marginBottom: 4 },
  barLabel: { fontSize: 10, color: '#9ca3af', textTransform: 'uppercase', letterSpacing: 0.4 },
  barBg: {
    height: 8,
    backgroundColor: '#f3f4f6',
    borderRadius: 999,
    overflow: 'hidden',
  },
  barFill: { height: '100%', borderRadius: 999 },
  warnBox: {
    backgroundColor: '#fffbeb',
    borderColor: '#fde68a',
    borderWidth: 1,
    borderRadius: 8,
    padding: 8,
    marginTop: 12,
  },
  warnText: { color: '#a16207', fontSize: 11 },
  btnRow: { flexDirection: 'row', gap: 8, marginTop: 12 },
  btn: {
    flex: 1,
    paddingVertical: 10,
    borderRadius: 10,
    alignItems: 'center',
    minHeight: 44,
    justifyContent: 'center',
  },
  btnGhost: { backgroundColor: '#f3f4f6', borderWidth: 1, borderColor: '#e5e7eb' },
  btnGhostText: { color: '#374151', fontWeight: '600', fontSize: 12 },
  btnCyan: { backgroundColor: '#06b6d4' },
  btnCyanText: { color: '#ffffff', fontWeight: '700', fontSize: 12 },
  note: { fontSize: 10, color: '#9ca3af', marginTop: 10, lineHeight: 14 },
});

export default HlwaScreen;
