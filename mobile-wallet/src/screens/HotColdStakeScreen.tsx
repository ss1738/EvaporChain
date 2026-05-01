/**
 * HotColdStakeScreen — Two-temperature validator stake equilibrium.
 *
 * Mirrors extension/src/components/HotColdStakeCard.tsx data flow as
 * a full screen (mobile UX is screen-driven, not card-stacked).
 *
 * Per research/INVENTION_STACK.md §4.2 / crates/evaporchain-hot-cold-stake:
 *   - Hot stake decays fast (default λ = 100 epochs), participates in
 *     block production, earns refresh credit on every block.
 *   - Cold stake decays slow (default λ = 10 000 epochs), held as
 *     long-term skin in the game.
 *   - Validators promote cold→hot (instant) or demote hot→cold (subject
 *     to a chain-set cool-down enforced at the caller).
 *
 * Endpoints (api.rs L4948-4987, all PURE COMPUTE — no signature, no
 * chain mutation):
 *   POST /api/hot_cold_stake/decay
 *   POST /api/hot_cold_stake/promote
 *   POST /api/hot_cold_stake/demote
 *
 * The "Equilibrium temperature" headline is the hot/(hot+cold) ratio
 * after decay — high = hot-dominant (active, volatile), low = cold-
 * dominant (reserve, stable).
 *
 * The actual on-chain promote/demote that mutates validator state will
 * land as a signed validator-stake transaction once that's wired
 * through evaporchain-execution. Until then the screen runs the
 * simulator in-place and writes the result back into the local pool
 * state so subsequent operations chain off it.
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
  RefreshControl,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { api } from '../utils/api';
import type {
  ChainStatus,
  HotColdStakeDecayResp,
  HotColdStakeMoveResp,
} from '../utils/api';
import { keystore } from '../utils/keystore';

// Default demo validator pools — match the api.rs doc example
// ("hot":1000000,"cold":5000000) and the crate's documented λ
// defaults (100 / 10 000 epochs).
const DEFAULT_HOT = 1_000_000;
const DEFAULT_COLD = 5_000_000;
const HOT_LAMBDA = 100;
const COLD_LAMBDA = 10_000;
const DEFAULT_MOVE = 100_000;

type Busy = 'idle' | 'decay' | 'promote' | 'demote';

const HotColdStakeScreen: React.FC = () => {
  const [chain, setChain] = useState<ChainStatus | null>(null);
  const [hot, setHot] = useState<number>(DEFAULT_HOT);
  const [cold, setCold] = useState<number>(DEFAULT_COLD);
  const [lastTouched, setLastTouched] = useState<number>(0);
  const [hotInput, setHotInput] = useState<string>(String(DEFAULT_HOT));
  const [coldInput, setColdInput] = useState<string>(String(DEFAULT_COLD));
  const [lastTouchedInput, setLastTouchedInput] = useState<string>('0');
  const [moveInput, setMoveInput] = useState<string>(String(DEFAULT_MOVE));

  const [decay, setDecay] = useState<HotColdStakeDecayResp | null>(null);
  const [busy, setBusy] = useState<Busy>('idle');
  const [error, setError] = useState<string | null>(null);
  const [lastMove, setLastMove] = useState<{
    direction: 'promote' | 'demote';
    moved: number;
  } | null>(null);
  const [loading, setLoading] = useState(true);

  // Use chain epoch when available; fall back to a safe non-zero value
  // (50 — matches the api.rs example) so the decay endpoint actually
  // shows movement on a fresh node.
  const currentEpoch = chain?.epoch ?? 50;

  const runDecay = useCallback(
    async (h: number, c: number, lt: number, ce: number) => {
      setBusy('decay');
      setError(null);
      try {
        const resp = await api.hotColdStakeDecay({
          hot: h,
          cold: c,
          hot_lambda_epochs: HOT_LAMBDA,
          cold_lambda_epochs: COLD_LAMBDA,
          last_touched_epoch: lt,
          current_epoch: ce,
        });
        setDecay(resp);
        if (resp.status !== 'ok') {
          setError('decay simulation failed');
        }
      } catch (e: unknown) {
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        setBusy('idle');
      }
    },
    [],
  );

  const loadChain = useCallback(async () => {
    try {
      // Pull chain epoch + (best-effort) account last-touched anchor.
      const [chainR, addrR] = await Promise.allSettled([
        api.getChainStatus(),
        keystore.getAddress(),
      ]);
      let epoch = 50;
      if (chainR.status === 'fulfilled') {
        setChain(chainR.value);
        epoch = chainR.value.epoch;
      }
      if (addrR.status === 'fulfilled' && addrR.value) {
        try {
          const detail = await api.getAccountDetail(addrR.value);
          const lt = detail.last_touched_epoch ?? 0;
          setLastTouched(lt);
          setLastTouchedInput(String(lt));
        } catch {
          /* fallback to 0 */
        }
      }
      // Run the initial decay against current state.
      await runDecay(hot, cold, lastTouched, epoch);
    } finally {
      setLoading(false);
    }
    // hot/cold/lastTouched are intentionally captured at mount; the
    // user re-runs decay manually after editing the inputs.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [runDecay]);

  useEffect(() => {
    loadChain();
  }, [loadChain]);

  const applyMove = (
    resp: HotColdStakeMoveResp,
    direction: 'promote' | 'demote',
  ) => {
    if (
      resp.status === 'ok' &&
      typeof resp.hot_after === 'number' &&
      typeof resp.cold_after === 'number'
    ) {
      // The endpoint already ran decay-then-move internally, so we
      // anchor the local state at the current epoch and adopt the
      // post-move pools verbatim.
      setHot(resp.hot_after);
      setCold(resp.cold_after);
      setHotInput(String(resp.hot_after));
      setColdInput(String(resp.cold_after));
      setLastTouched(currentEpoch);
      setLastTouchedInput(String(currentEpoch));
      setDecay({
        status: 'ok',
        hot_after: resp.hot_after,
        cold_after: resp.cold_after,
        total_after: resp.total_after ?? resp.hot_after + resp.cold_after,
        epochs_elapsed: 0,
      });
      const moved =
        direction === 'promote' ? resp.promoted ?? 0 : resp.demoted ?? 0;
      setLastMove({ direction, moved });
    } else {
      setError(resp.detail || `${direction} failed`);
    }
  };

  const parseInputs = (): { h: number; c: number; lt: number } | null => {
    const h = parseInt(hotInput, 10);
    const c = parseInt(coldInput, 10);
    const lt = parseInt(lastTouchedInput, 10);
    if (!Number.isFinite(h) || h < 0) return null;
    if (!Number.isFinite(c) || c < 0) return null;
    if (!Number.isFinite(lt) || lt < 0) return null;
    return { h, c, lt };
  };

  const handleApplyDecay = async () => {
    const p = parseInputs();
    if (!p) {
      setError('invalid pool inputs');
      return;
    }
    setHot(p.h);
    setCold(p.c);
    setLastTouched(p.lt);
    await runDecay(p.h, p.c, p.lt, currentEpoch);
  };

  const handlePromote = async () => {
    const p = parseInputs();
    const amount = parseInt(moveInput, 10);
    if (!p || !Number.isFinite(amount) || amount <= 0) {
      setError('invalid move amount');
      return;
    }
    setBusy('promote');
    setError(null);
    try {
      const resp = await api.hotColdStakePromote({
        hot: p.h,
        cold: p.c,
        hot_lambda_epochs: HOT_LAMBDA,
        cold_lambda_epochs: COLD_LAMBDA,
        last_touched_epoch: p.lt,
        current_epoch: currentEpoch,
        amount,
      });
      applyMove(resp, 'promote');
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy('idle');
    }
  };

  const handleDemote = async () => {
    const p = parseInputs();
    const amount = parseInt(moveInput, 10);
    if (!p || !Number.isFinite(amount) || amount <= 0) {
      setError('invalid move amount');
      return;
    }
    setBusy('demote');
    setError(null);
    try {
      const resp = await api.hotColdStakeDemote({
        hot: p.h,
        cold: p.c,
        hot_lambda_epochs: HOT_LAMBDA,
        cold_lambda_epochs: COLD_LAMBDA,
        last_touched_epoch: p.lt,
        current_epoch: currentEpoch,
        amount,
      });
      applyMove(resp, 'demote');
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy('idle');
    }
  };

  if (loading) {
    return (
      <View style={styles.centered}>
        <ActivityIndicator size="large" color="#f59e0b" />
      </View>
    );
  }

  const hotAfter = decay?.hot_after ?? hot;
  const coldAfter = decay?.cold_after ?? cold;
  const totalAfter = decay?.total_after ?? hotAfter + coldAfter;
  const tempRatio = totalAfter > 0 ? hotAfter / totalAfter : 0;
  const tempPct = Math.max(0, Math.min(100, tempRatio * 100));
  const tempLabel =
    tempPct > 66
      ? 'Hot-dominant'
      : tempPct > 33
        ? 'Balanced'
        : 'Cold-dominant';
  const tempPalette =
    tempPct > 66
      ? { bg: '#fffbeb', bd: '#fde68a', fg: '#f59e0b' }
      : tempPct > 33
        ? { bg: '#ecfdf5', bd: '#a7f3d0', fg: '#22c55e' }
        : { bg: '#ecfeff', bd: '#a5f3fc', fg: '#06b6d4' };

  const hotDelta = hotAfter - hot;
  const coldDelta = coldAfter - cold;

  return (
    <SafeAreaView style={styles.container} edges={['bottom']}>
      <ScrollView
        contentContainerStyle={styles.scroll}
        refreshControl={
          <RefreshControl
            refreshing={false}
            onRefresh={loadChain}
            tintColor="#f59e0b"
          />
        }
      >
        <Text style={styles.intro}>
          Two-temperature stake equilibrium — hot earns rewards, cold preserves capital.
        </Text>

        {error ? (
          <View style={styles.errorBox}>
            <Text style={styles.errorText}>{error}</Text>
          </View>
        ) : null}

        {/* Headline: equilibrium temperature */}
        <View style={styles.card}>
          <View style={styles.headlineRow}>
            <Text style={styles.label}>Equilibrium temperature</Text>
            <View
              style={[
                styles.pill,
                { backgroundColor: tempPalette.bg, borderColor: tempPalette.bd },
              ]}
            >
              <Text style={[styles.pillText, { color: tempPalette.fg }]}>
                {tempLabel}
              </Text>
            </View>
          </View>
          <View style={styles.bigRow}>
            <Text style={styles.bigVal}>{tempPct.toFixed(1)}%</Text>
            <Text style={styles.bigUnit}>hot fraction</Text>
          </View>

          {/* Hot/cold split bar */}
          <View style={styles.barTrack}>
            <View
              style={[
                styles.barHot,
                { width: `${tempPct}%` },
              ]}
            />
            <View
              style={[
                styles.barCold,
                { width: `${100 - tempPct}%` },
              ]}
            />
          </View>

          <View style={styles.gridFooter}>
            <Text style={styles.subtle}>
              λ {HOT_LAMBDA} / {COLD_LAMBDA.toLocaleString()} epochs
            </Text>
            <Text style={styles.subtle}>
              Epoch {currentEpoch.toLocaleString()}
            </Text>
          </View>
        </View>

        {/* Pool stats (post-decay) */}
        <View style={styles.card}>
          <Text style={styles.label}>Post-decay pools</Text>
          <View style={styles.statsRow}>
            <PoolStat
              label="Hot"
              value={hotAfter.toLocaleString()}
              color="#f59e0b"
              delta={hotDelta}
            />
            <PoolStat
              label="Cold"
              value={coldAfter.toLocaleString()}
              color="#06b6d4"
              delta={coldDelta}
            />
          </View>
          <View style={styles.statsRow}>
            <PoolStat
              label="Total"
              value={totalAfter.toLocaleString()}
            />
            <PoolStat
              label="Last touched"
              value={`Ep ${lastTouched.toLocaleString()}`}
            />
          </View>
        </View>

        {/* Pool inputs */}
        <View style={styles.card}>
          <Text style={styles.label}>Pool inputs</Text>
          <Field label="Hot stake (EVAP)">
            <TextInput
              style={styles.input}
              value={hotInput}
              onChangeText={setHotInput}
              keyboardType="numeric"
              placeholder="1000000"
              placeholderTextColor="#9ca3af"
            />
          </Field>
          <Field label="Cold stake (EVAP)">
            <TextInput
              style={styles.input}
              value={coldInput}
              onChangeText={setColdInput}
              keyboardType="numeric"
              placeholder="5000000"
              placeholderTextColor="#9ca3af"
            />
          </Field>
          <Field label="Last touched epoch">
            <TextInput
              style={styles.input}
              value={lastTouchedInput}
              onChangeText={setLastTouchedInput}
              keyboardType="numeric"
              placeholder="0"
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
          <TouchableOpacity
            style={[
              styles.btn,
              styles.btnApply,
              busy !== 'idle' && { opacity: 0.5 },
            ]}
            disabled={busy !== 'idle'}
            onPress={handleApplyDecay}
            activeOpacity={0.7}
          >
            <Text style={styles.btnApplyText}>
              {busy === 'decay' ? '…' : 'Apply decay'}
            </Text>
          </TouchableOpacity>
        </View>

        {/* Move-stake controls */}
        <View style={styles.card}>
          <Text style={styles.label}>Move stake hot ↔ cold</Text>
          <Field label="Amount (EVAP)">
            <TextInput
              style={styles.input}
              value={moveInput}
              onChangeText={setMoveInput}
              keyboardType="numeric"
              placeholder="100000"
              placeholderTextColor="#9ca3af"
            />
          </Field>
          <View style={styles.moveRow}>
            <TouchableOpacity
              style={[
                styles.btn,
                styles.btnHot,
                busy !== 'idle' && { opacity: 0.5 },
              ]}
              disabled={busy !== 'idle'}
              onPress={handlePromote}
              activeOpacity={0.7}
            >
              <Text style={styles.btnHotText}>
                {busy === 'promote' ? '…' : 'Cold → Hot'}
              </Text>
            </TouchableOpacity>
            <TouchableOpacity
              style={[
                styles.btn,
                styles.btnCold,
                busy !== 'idle' && { opacity: 0.5 },
              ]}
              disabled={busy !== 'idle'}
              onPress={handleDemote}
              activeOpacity={0.7}
            >
              <Text style={styles.btnColdText}>
                {busy === 'demote' ? '…' : 'Hot → Cold'}
              </Text>
            </TouchableOpacity>
          </View>
          {lastMove ? (
            <Text style={styles.lastMove}>
              Last move:{' '}
              <Text
                style={{
                  color:
                    lastMove.direction === 'promote' ? '#f59e0b' : '#06b6d4',
                  fontWeight: '600',
                }}
              >
                {lastMove.direction === 'promote' ? 'Cold → Hot' : 'Hot → Cold'}
              </Text>{' '}
              <Text style={styles.bold}>{lastMove.moved.toLocaleString()}</Text>
            </Text>
          ) : null}
        </View>

        <View style={styles.note}>
          <Text style={styles.noteText}>
            Pure-compute simulator over the substrate at /api/hot_cold_stake/
            &#123;decay,promote,demote&#125;. No signature, no chain mutation —
            promote/demote that mutates validator state will land as a signed
            validator-stake tx once wired through execution.
          </Text>
        </View>
      </ScrollView>
    </SafeAreaView>
  );
};

const PoolStat: React.FC<{
  label: string;
  value: string;
  color?: string;
  delta?: number;
}> = ({ label, value, color, delta }) => {
  const showDelta = typeof delta === 'number' && Math.abs(delta) >= 1;
  return (
    <View style={{ flex: 1 }}>
      <Text style={styles.statLabel}>{label}</Text>
      <View style={styles.statValueRow}>
        <Text style={[styles.statValue, color ? { color } : null]}>{value}</Text>
        {showDelta ? (
          <Text
            style={[
              styles.statDelta,
              { color: (delta ?? 0) >= 0 ? '#22c55e' : '#ef4444' },
            ]}
          >
            {(delta ?? 0) >= 0 ? '+' : ''}
            {(delta ?? 0).toLocaleString()}
          </Text>
        ) : null}
      </View>
    </View>
  );
};

const Field: React.FC<{ label: string; children: React.ReactNode }> = ({
  label,
  children,
}) => (
  <View style={{ marginBottom: 8 }}>
    <Text style={styles.fieldLabel}>{label}</Text>
    {children}
  </View>
);

const styles = StyleSheet.create({
  container: { flex: 1, backgroundColor: '#f9fafb' },
  centered: {
    flex: 1,
    alignItems: 'center',
    justifyContent: 'center',
    backgroundColor: '#f9fafb',
  },
  scroll: { padding: 16, paddingBottom: 40 },
  intro: { fontSize: 12, color: '#6b7280', marginBottom: 12, lineHeight: 16 },
  errorBox: {
    backgroundColor: '#fef2f2',
    borderColor: '#fecaca',
    borderWidth: 1,
    borderRadius: 10,
    padding: 10,
    marginBottom: 12,
  },
  errorText: { fontSize: 11, color: '#ef4444' },
  card: {
    backgroundColor: '#ffffff',
    borderRadius: 14,
    padding: 16,
    marginBottom: 12,
    borderWidth: 1,
    borderColor: '#e5e7eb',
  },
  label: {
    fontSize: 11,
    color: '#6b7280',
    fontWeight: '600',
    textTransform: 'uppercase',
    letterSpacing: 0.5,
    marginBottom: 6,
  },
  headlineRow: {
    flexDirection: 'row',
    justifyContent: 'space-between',
    alignItems: 'center',
    marginBottom: 4,
  },
  pill: {
    paddingHorizontal: 10,
    paddingVertical: 4,
    borderRadius: 999,
    borderWidth: 1,
  },
  pillText: { fontSize: 10, fontWeight: '600' },
  bigRow: { flexDirection: 'row', alignItems: 'baseline', gap: 6 },
  bigVal: { fontSize: 28, fontWeight: '700', color: '#111827' },
  bigUnit: { fontSize: 12, color: '#9ca3af' },
  barTrack: {
    flexDirection: 'row',
    height: 8,
    backgroundColor: '#f3f4f6',
    borderRadius: 999,
    overflow: 'hidden',
    marginTop: 12,
    borderWidth: 1,
    borderColor: '#e5e7eb',
  },
  barHot: { height: '100%', backgroundColor: '#f59e0b' },
  barCold: { height: '100%', backgroundColor: '#06b6d4' },
  gridFooter: {
    flexDirection: 'row',
    justifyContent: 'space-between',
    marginTop: 12,
    paddingTop: 10,
    borderTopWidth: 1,
    borderTopColor: '#f3f4f6',
  },
  subtle: { fontSize: 11, color: '#6b7280' },
  statsRow: { flexDirection: 'row', gap: 16, marginBottom: 8 },
  statLabel: {
    fontSize: 10,
    color: '#9ca3af',
    textTransform: 'uppercase',
    letterSpacing: 0.4,
  },
  statValueRow: {
    flexDirection: 'row',
    alignItems: 'baseline',
    gap: 6,
    marginTop: 2,
  },
  statValue: { fontSize: 16, fontWeight: '700', color: '#111827' },
  statDelta: { fontSize: 10, fontWeight: '600' },
  fieldLabel: {
    fontSize: 11,
    color: '#6b7280',
    fontWeight: '500',
    marginBottom: 4,
  },
  input: {
    backgroundColor: '#f9fafb',
    borderColor: '#e5e7eb',
    borderWidth: 1,
    borderRadius: 8,
    paddingHorizontal: 10,
    paddingVertical: 10,
    fontSize: 13,
    color: '#111827',
  },
  inputDisabled: { backgroundColor: '#f3f4f6', color: '#6b7280' },
  btn: {
    paddingVertical: 12,
    borderRadius: 10,
    alignItems: 'center',
    justifyContent: 'center',
    borderWidth: 1,
  },
  btnApply: {
    backgroundColor: '#fffbeb',
    borderColor: '#fde68a',
    marginTop: 4,
  },
  btnApplyText: { color: '#f59e0b', fontWeight: '600', fontSize: 13 },
  moveRow: { flexDirection: 'row', gap: 8, marginTop: 4 },
  btnHot: {
    flex: 1,
    backgroundColor: '#fffbeb',
    borderColor: '#fde68a',
  },
  btnHotText: { color: '#f59e0b', fontWeight: '600', fontSize: 13 },
  btnCold: {
    flex: 1,
    backgroundColor: '#ecfeff',
    borderColor: '#a5f3fc',
  },
  btnColdText: { color: '#06b6d4', fontWeight: '600', fontSize: 13 },
  lastMove: { fontSize: 11, color: '#6b7280', marginTop: 8 },
  bold: { fontWeight: '700', color: '#111827' },
  note: {
    backgroundColor: '#fffbeb',
    borderColor: '#fde68a',
    borderWidth: 1,
    borderRadius: 10,
    padding: 12,
  },
  noteText: { fontSize: 11, color: '#92400e', lineHeight: 16 },
});

export default HotColdStakeScreen;
