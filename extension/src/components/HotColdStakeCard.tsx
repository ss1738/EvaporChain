import { useEffect, useMemo, useState } from "react";
import { useWallet } from "@/hooks/useWallet";
import {
  api,
  type HotColdStakeDecayResp,
  type HotColdStakeMoveResp,
} from "@/utils/api";

/**
 * Hot/Cold Stake — two-temperature validator equilibrium card.
 *
 * Per research/INVENTION_STACK.md §4.2 / crates/evaporchain-hot-cold-stake:
 *
 *   - Hot stake decays fast (default λ = 100 epochs), participates in
 *     block production, earns refresh credit on every block.
 *   - Cold stake decays slow (default λ = 10 000 epochs), held as
 *     long-term skin in the game.
 *   - Validators promote cold→hot (instant) or demote hot→cold (subject
 *     to a chain-set cool-down enforced at the caller).
 *
 * Endpoints (api.rs L4921-4987):
 *   POST /api/hot_cold_stake/decay   — apply both decays from
 *        last_touched_epoch to current_epoch.
 *   POST /api/hot_cold_stake/promote — simulate cold→hot of `amount`.
 *   POST /api/hot_cold_stake/demote  — simulate hot→cold of `amount`.
 *
 * All three are PURE COMPUTE — no signature, no chain mutation. This
 * card surfaces the substrate so a validator (or any user inspecting
 * the substrate) can see how their hot/cold pools would settle after
 * decay and after a promote/demote. The actual on-chain promote/demote
 * would land as a signed validator-stake transaction once that's wired
 * through evaporchain-execution. Until then the card runs the
 * simulator in-place and writes the result back into the local pool
 * state so subsequent operations chain off it.
 *
 * The "Equilibrium temperature" headline is the hot/(hot+cold) ratio
 * after decay — a high value means a hot-dominant validator (active,
 * volatile), a low value means cold-dominant (reserve, stable).
 */
export function HotColdStakeCard() {
  const { chainStatus, refreshChainStatus } = useWallet();

  // Default demo validator pools — match the api.rs doc example
  // ("hot":1000000,"cold":5000000) and the crate's documented λ
  // defaults (100 / 10 000 epochs).
  const [hot, setHot] = useState(1_000_000);
  const [cold, setCold] = useState(5_000_000);
  const [lastTouched, setLastTouched] = useState(0);
  const hotLambda = 100;
  const coldLambda = 10_000;

  const [decay, setDecay] = useState<HotColdStakeDecayResp | null>(null);
  const [moveAmount, setMoveAmount] = useState(100_000);
  const [busy, setBusy] = useState<"idle" | "decay" | "promote" | "demote">(
    "idle",
  );
  const [error, setError] = useState<string | null>(null);
  const [lastMove, setLastMove] = useState<{
    direction: "promote" | "demote";
    moved: number;
  } | null>(null);

  // Use chain epoch when available; fall back to a safe non-zero value
  // (50 — matches the api.rs example) so the decay endpoint actually
  // shows movement on a fresh node.
  const currentEpoch = chainStatus?.epoch ?? 50;

  const fetchDecay = useMemo(
    () => async () => {
      setBusy("decay");
      setError(null);
      try {
        const resp = await api.hotColdStakeDecay({
          hot,
          cold,
          hot_lambda_epochs: hotLambda,
          cold_lambda_epochs: coldLambda,
          last_touched_epoch: lastTouched,
          current_epoch: currentEpoch,
        });
        setDecay(resp);
        if (resp.status !== "ok") {
          setError("decay simulation failed");
        }
      } catch (e: unknown) {
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        setBusy("idle");
      }
    },
    [hot, cold, lastTouched, currentEpoch],
  );

  useEffect(() => {
    refreshChainStatus();
  }, [refreshChainStatus]);

  useEffect(() => {
    fetchDecay();
  }, [fetchDecay]);

  const applyMove = (resp: HotColdStakeMoveResp, direction: "promote" | "demote") => {
    if (
      resp.status === "ok" &&
      typeof resp.hot_after === "number" &&
      typeof resp.cold_after === "number"
    ) {
      // The endpoint already ran decay-then-move internally, so we
      // anchor the local state at the current epoch and adopt the
      // post-move pools verbatim.
      setHot(resp.hot_after);
      setCold(resp.cold_after);
      setLastTouched(currentEpoch);
      setDecay({
        status: "ok",
        hot_after: resp.hot_after,
        cold_after: resp.cold_after,
        total_after: resp.total_after ?? resp.hot_after + resp.cold_after,
        epochs_elapsed: 0,
      });
      const moved = direction === "promote" ? resp.promoted ?? 0 : resp.demoted ?? 0;
      setLastMove({ direction, moved });
    } else {
      setError(resp.detail || `${direction} failed`);
    }
  };

  const handlePromote = async () => {
    if (moveAmount <= 0) return;
    setBusy("promote");
    setError(null);
    try {
      const resp = await api.hotColdStakePromote({
        hot,
        cold,
        hot_lambda_epochs: hotLambda,
        cold_lambda_epochs: coldLambda,
        last_touched_epoch: lastTouched,
        current_epoch: currentEpoch,
        amount: moveAmount,
      });
      applyMove(resp, "promote");
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy("idle");
    }
  };

  const handleDemote = async () => {
    if (moveAmount <= 0) return;
    setBusy("demote");
    setError(null);
    try {
      const resp = await api.hotColdStakeDemote({
        hot,
        cold,
        hot_lambda_epochs: hotLambda,
        cold_lambda_epochs: coldLambda,
        last_touched_epoch: lastTouched,
        current_epoch: currentEpoch,
        amount: moveAmount,
      });
      applyMove(resp, "demote");
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy("idle");
    }
  };

  const hotAfter = decay?.hot_after ?? hot;
  const coldAfter = decay?.cold_after ?? cold;
  const totalAfter = decay?.total_after ?? hotAfter + coldAfter;
  // Equilibrium temperature: hot fraction. 0 = pure cold, 1 = pure hot.
  const tempRatio = totalAfter > 0 ? hotAfter / totalAfter : 0;
  const tempPct = Math.max(0, Math.min(100, tempRatio * 100));
  const tempLabel =
    tempPct > 66
      ? "Hot-dominant"
      : tempPct > 33
      ? "Balanced"
      : "Cold-dominant";

  return (
    <div className="rounded-lg bg-evap-surface border border-evap-border p-3 space-y-3">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <span className="text-[10px] font-semibold px-2 py-0.5 rounded-full bg-evap-purple/15 text-evap-purple border border-evap-purple/40">
            Stake
          </span>
          <span className="text-xs font-medium text-zinc-300 uppercase tracking-wide">
            Hot / Cold equilibrium
          </span>
        </div>
        <span className="text-[10px] px-2 py-0.5 rounded-full bg-evap-bg text-zinc-400 border border-evap-border">
          λ {hotLambda} / {coldLambda.toLocaleString()} ep
        </span>
      </div>

      {error && (
        <div className="px-2 py-1.5 rounded bg-evap-red/10 border border-evap-red/30">
          <p className="text-xs text-evap-red">{error}</p>
        </div>
      )}

      {/* Headline: equilibrium temperature */}
      <div>
        <div className="flex items-baseline gap-2">
          <span className="text-2xl font-bold text-zinc-100 tabular-nums">
            {tempPct.toFixed(1)}%
          </span>
          <span className="text-xs text-zinc-500">hot fraction</span>
          <span
            className={`ml-auto text-[10px] font-semibold px-2 py-0.5 rounded-full border ${
              tempPct > 66
                ? "bg-evap-amber/15 text-evap-amber border-evap-amber/40"
                : tempPct > 33
                ? "bg-evap-green/15 text-evap-green border-evap-green/40"
                : "bg-evap-cyan/15 text-evap-cyan border-evap-cyan/40"
            }`}
          >
            {tempLabel}
          </span>
        </div>
        {/* Hot/cold split bar */}
        <div className="mt-2 w-full h-2 rounded-full overflow-hidden flex bg-evap-bg border border-evap-border">
          <div
            className="h-full bg-evap-amber transition-all"
            style={{ width: `${tempPct}%` }}
            title={`Hot ${hotAfter.toLocaleString()}`}
          />
          <div
            className="h-full bg-evap-cyan transition-all"
            style={{ width: `${100 - tempPct}%` }}
            title={`Cold ${coldAfter.toLocaleString()}`}
          />
        </div>
      </div>

      {/* Pool stats (post-decay) */}
      <div className="grid grid-cols-2 gap-3">
        <PoolStat
          label="Hot"
          value={hotAfter.toLocaleString()}
          color="text-evap-amber"
          delta={hotAfter - hot}
        />
        <PoolStat
          label="Cold"
          value={coldAfter.toLocaleString()}
          color="text-evap-cyan"
          delta={coldAfter - cold}
        />
        <PoolStat
          label="Last touched"
          value={`Epoch ${lastTouched.toLocaleString()}`}
        />
        <PoolStat
          label="Current epoch"
          value={`Epoch ${currentEpoch.toLocaleString()}`}
        />
      </div>

      {/* Move-stake controls */}
      <div className="rounded bg-evap-bg/60 border border-evap-border p-2 space-y-2">
        <div className="flex items-center gap-2">
          <span className="text-xs text-zinc-400 uppercase tracking-wider">
            Move stake hot ↔ cold
          </span>
        </div>
        <div className="flex items-center gap-2">
          <input
            type="number"
            min={1}
            value={moveAmount}
            onChange={(e) => setMoveAmount(Math.max(0, Number(e.target.value) || 0))}
            className="flex-1 px-2 py-1 rounded bg-evap-surface border border-evap-border text-xs text-zinc-200 tabular-nums focus:outline-none focus:border-evap-cyan/60"
            placeholder="amount"
          />
          <span className="text-[10px] text-zinc-500">EVAP</span>
        </div>
        <div className="flex gap-2">
          <button
            onClick={handlePromote}
            disabled={busy !== "idle" || moveAmount <= 0}
            className="flex-1 py-1.5 rounded text-xs font-medium bg-evap-amber/15 text-evap-amber border border-evap-amber/40 hover:border-evap-amber/70 transition disabled:opacity-50"
            title="Move cold → hot (active block-production stake)"
          >
            {busy === "promote" ? "…" : "Cold → Hot"}
          </button>
          <button
            onClick={handleDemote}
            disabled={busy !== "idle" || moveAmount <= 0}
            className="flex-1 py-1.5 rounded text-xs font-medium bg-evap-cyan/15 text-evap-cyan border border-evap-cyan/40 hover:border-evap-cyan/70 transition disabled:opacity-50"
            title="Move hot → cold (long-term reserve)"
          >
            {busy === "demote" ? "…" : "Hot → Cold"}
          </button>
        </div>
        {lastMove && (
          <p className="text-[10px] text-zinc-500 leading-snug">
            Last move:{" "}
            <span
              className={
                lastMove.direction === "promote"
                  ? "text-evap-amber"
                  : "text-evap-cyan"
              }
            >
              {lastMove.direction === "promote" ? "Cold → Hot" : "Hot → Cold"}
            </span>{" "}
            <span className="tabular-nums">{lastMove.moved.toLocaleString()}</span>
          </p>
        )}
      </div>

      <p className="text-[10px] text-zinc-500 leading-snug">
        Pure-compute simulator over the substrate at{" "}
        <span className="font-mono">/api/hot_cold_stake/&#123;decay,promote,demote&#125;</span>.
        No signature, no chain mutation — promote/demote that mutates
        validator state will land as a signed validator-stake tx once
        wired through execution.
      </p>
    </div>
  );
}

function PoolStat({
  label,
  value,
  color,
  delta,
}: {
  label: string;
  value: string;
  color?: string;
  delta?: number;
}) {
  const showDelta = typeof delta === "number" && Math.abs(delta) >= 1;
  return (
    <div>
      <p className="text-[10px] text-zinc-500 uppercase tracking-wider">{label}</p>
      <div className="flex items-baseline gap-1 mt-0.5">
        <span className={`text-sm font-semibold tabular-nums ${color ?? "text-zinc-200"}`}>
          {value}
        </span>
        {showDelta && (
          <span
            className={`text-[10px] tabular-nums ${
              (delta ?? 0) >= 0 ? "text-evap-green" : "text-evap-red"
            }`}
          >
            {(delta ?? 0) >= 0 ? "+" : ""}
            {(delta ?? 0).toLocaleString()}
          </span>
        )}
      </div>
    </div>
  );
}
