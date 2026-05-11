import { useEffect, useState } from "react";
import { RotateCw } from "lucide-react";
import { useWallet } from "@/hooks/useWallet";
import { api, type ForkChoiceAttractor } from "@/utils/api";

/**
 * Chain governance: fork-choice mode amendment.
 *
 * Endpoints (api.rs):
 *   GET  /api/governance/fork_choice_mode  — L1951 — returns
 *        { fork_choice_mode: "mcc"|"singh_attractor", attractors: [{center,
 *          basin_radius}], detail }
 *   POST /api/governance/fork_choice_mode  — L1912 — accepts
 *        { mode, attractors?, endorser_stakes[], required_stake }
 *        Quorum: sum(endorser_stakes) >= required_stake.
 *
 * The POST does NOT take signature/public_key — the chain validates
 * quorum from raw stake numbers. We still call signTransaction to
 * stamp the local tx history, but only the body is broadcast.
 *
 * The endorse form is gated behind import.meta.env.DEV — heavy op.
 */
export function GovernanceScreen() {
  const {
    forkChoiceMode,
    refreshForkChoiceMode,
    signTransaction,
    setView,
    setNotification,
    setError,
    notification,
    error,
  } = useWallet();

  const [submitting, setSubmitting] = useState(false);
  const [targetMode, setTargetMode] = useState<string>("singh_attractor");
  const [attractorJson, setAttractorJson] = useState<string>(
    JSON.stringify([{ center: 1000, basin_radius: 200 }], null, 2),
  );
  const [stake, setStake] = useState<string>("1000");
  const [requiredStake, setRequiredStake] = useState<string>("1500");

  useEffect(() => {
    refreshForkChoiceMode();
  }, [refreshForkChoiceMode]);

  const proGated = import.meta.env.DEV;
  const currentMode = forkChoiceMode?.fork_choice_mode ?? "—";
  const currentAttractors = forkChoiceMode?.attractors ?? [];

  const handleEndorse = async () => {
    setError(null);
    let attractors: ForkChoiceAttractor[] | undefined;
    try {
      const parsed = JSON.parse(attractorJson);
      if (!Array.isArray(parsed)) throw new Error("attractors must be a JSON array");
      attractors = parsed.map((a: any) => ({
        center: Number(a.center),
        basin_radius: Number(a.basin_radius),
      }));
    } catch (e: any) {
      setError(`Invalid attractor JSON: ${e.message}`);
      return;
    }

    const stakeN = parseInt(stake);
    const reqN = parseInt(requiredStake);
    if (!Number.isFinite(stakeN) || stakeN <= 0) {
      setError("Stake must be > 0");
      return;
    }
    if (!Number.isFinite(reqN) || reqN <= 0) {
      setError("Required stake must be > 0");
      return;
    }

    setSubmitting(true);
    try {
      // Sign body locally for tx history; chain enforces stake quorum, no
      // signature field exists in ForkChoiceAmendReq.
      await signTransaction(
        JSON.stringify({
          type: "governance_fork_choice",
          mode: targetMode,
          attractors,
          endorser_stakes: [stakeN],
          required_stake: reqN,
        }),
      );

      const resp = await api.amendForkChoiceMode({
        mode: targetMode,
        attractors: targetMode === "singh_attractor" ? attractors : undefined,
        endorser_stakes: [stakeN],
        required_stake: reqN,
      });

      if (resp.status === "amended") {
        setNotification(`Fork-choice → ${resp.fork_choice_mode}`);
        await refreshForkChoiceMode();
      } else {
        setError(resp.detail || "Amendment rejected");
      }
    } catch (e: any) {
      setError(e.message ?? "Submission failed");
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="flex flex-col h-full bg-evap-bg">
      {/* Header */}
      <div className="flex items-center gap-3 px-4 py-3 border-b border-evap-border">
        <button
          onClick={() => setView("settings")}
          className="text-zinc-400 hover:text-zinc-200 transition text-sm"
        >
          ←
        </button>
        <div className="flex-1">
          <h1 className="text-sm font-semibold text-zinc-200">Chain Governance</h1>
          <p className="text-xs text-zinc-500">Fork-choice rule + Singh-Attractor basin set.</p>
        </div>
        <button
          onClick={() => refreshForkChoiceMode()}
          className="text-xs px-2 py-1 rounded text-evap-cyan border border-evap-cyan/30 hover:border-evap-cyan/60 transition"
        ><RotateCw className="w-3.5 h-3.5" strokeWidth={1.5} /></button>
      </div>

      <div className="flex-1 overflow-y-auto">
        {notification && (
          <div className="mx-4 mt-3 px-3 py-2 rounded-lg bg-evap-green/10 border border-evap-green/30">
            <p className="text-xs text-evap-green">{notification}</p>
          </div>
        )}
        {error && (
          <div className="mx-4 mt-3 px-3 py-2 rounded-lg bg-evap-red/10 border border-evap-red/30">
            <p className="text-xs text-evap-red">{error}</p>
          </div>
        )}

        {/* Current mode card */}
        <div className="mx-4 mt-3 px-3 py-3 rounded-lg bg-evap-surface border border-evap-border">
          <p className="text-xs text-zinc-400 font-semibold mb-2 uppercase tracking-wider">
            Active fork-choice mode
          </p>
          <div className="flex items-baseline gap-2">
            <span className="text-2xl font-bold text-evap-cyan tabular-nums">
              {currentMode === "mcc" ? "MCC" : currentMode === "singh_attractor" ? "Singh-Attractor" : currentMode}
            </span>
          </div>
          {forkChoiceMode?.detail && (
            <p className="text-[9px] text-zinc-500 mt-2 leading-snug">
              {forkChoiceMode.detail}
            </p>
          )}
        </div>

        {/* Mode explainer cards */}
        <div className="mx-4 mt-3 grid grid-cols-1 gap-2">
          <ModeCard
            name="MCC"
            id="mcc"
            active={currentMode === "mcc"}
            blurb="Most-Cumulative-Choice — vanilla heaviest-chain selector. Default genesis rule. Cheap to verify; no basin attractors."
          />
          <ModeCard
            name="Singh-Attractor"
            id="singh_attractor"
            active={currentMode === "singh_attractor"}
            blurb="Pulls competing forks toward attractor centres weighted by basin_radius — provably converges to a unique tip when basins are non-overlapping."
          />
        </div>

        {/* Active attractor set */}
        {currentMode === "singh_attractor" && (
          <div className="mx-4 mt-3 px-3 py-3 rounded-lg bg-evap-surface border border-evap-border">
            <p className="text-xs text-zinc-400 font-semibold mb-2 uppercase tracking-wider">
              Attractor set ({currentAttractors.length})
            </p>
            {currentAttractors.length === 0 ? (
              <p className="text-xs text-zinc-600">No attractors configured.</p>
            ) : (
              <div className="space-y-1">
                {currentAttractors.map((a, i) => (
                  <div
                    key={i}
                    className="flex justify-between items-center px-2 py-1.5 rounded bg-evap-bg border border-evap-border"
                  >
                    <span className="text-xs text-zinc-400">#{i + 1}</span>
                    <div className="flex gap-3 text-xs tabular-nums">
                      <span>
                        <span className="text-zinc-500">centre </span>
                        <span className="text-zinc-200">{a.center.toLocaleString()}</span>
                      </span>
                      <span>
                        <span className="text-zinc-500">basin </span>
                        <span className="text-zinc-200">{a.basin_radius.toLocaleString()}</span>
                      </span>
                    </div>
                  </div>
                ))}
              </div>
            )}
          </div>
        )}

        {/* Pro endorse form */}
        <div className="mx-4 mt-4 mb-4 rounded-lg bg-evap-surface border border-evap-border">
          <div className="flex items-center justify-between px-3 py-2 border-b border-evap-border">
            <p className="text-xs font-semibold uppercase tracking-wider text-zinc-400">
              Endorse amendment
            </p>
            <span className="text-[9px] px-1.5 py-0.5 rounded-full bg-evap-purple/10 text-evap-purple border border-evap-purple/30">
              Pro
            </span>
          </div>

          {!proGated ? (
            <div className="px-3 py-4">
              <p className="text-xs text-zinc-500 leading-snug">
                Amendment endorsement requires Pro mode. This is a heavy
                stake-weighted operation; enable a dev build to access it.
              </p>
            </div>
          ) : (
            <div className="px-3 py-3 space-y-3">
              <Field label="Target mode">
                <select
                  value={targetMode}
                  onChange={(e) => setTargetMode(e.target.value)}
                  className="w-full bg-evap-bg border border-evap-border rounded px-2 py-1 text-xs text-zinc-200 focus:outline-none focus:border-evap-cyan/60"
                >
                  <option value="mcc">MCC</option>
                  <option value="singh_attractor">Singh-Attractor</option>
                </select>
              </Field>

              <Field label="Attractors (JSON)">
                <textarea
                  value={attractorJson}
                  onChange={(e) => setAttractorJson(e.target.value)}
                  rows={4}
                  className="w-full bg-evap-bg border border-evap-border rounded px-2 py-1 text-xs font-mono text-zinc-200 focus:outline-none focus:border-evap-cyan/60 resize-y"
                />
              </Field>

              <div className="grid grid-cols-2 gap-2">
                <Field label="Your stake">
                  <input
                    type="number"
                    min="1"
                    value={stake}
                    onChange={(e) => setStake(e.target.value)}
                    className="w-full bg-evap-bg border border-evap-border rounded px-2 py-1 text-xs text-zinc-200 focus:outline-none focus:border-evap-cyan/60"
                  />
                </Field>
                <Field label="Required quorum">
                  <input
                    type="number"
                    min="1"
                    value={requiredStake}
                    onChange={(e) => setRequiredStake(e.target.value)}
                    className="w-full bg-evap-bg border border-evap-border rounded px-2 py-1 text-xs text-zinc-200 focus:outline-none focus:border-evap-cyan/60"
                  />
                </Field>
              </div>

              <button
                onClick={handleEndorse}
                disabled={submitting}
                className="w-full py-2 rounded-lg bg-evap-cyan text-black text-xs font-semibold hover:bg-evap-cyan/90 transition disabled:opacity-50"
              >
                {submitting ? "Broadcasting…" : "Sign & broadcast amendment"}
              </button>

              <p className="text-[9px] text-zinc-600 leading-snug">
                Note: the on-chain endpoint enforces quorum from raw stake
                numbers — this body is signed locally for your tx record but
                the chain does not require a signature.
              </p>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

function ModeCard({
  name,
  id,
  active,
  blurb,
}: {
  name: string;
  id: string;
  active: boolean;
  blurb: string;
}) {
  return (
    <div
      className={`px-3 py-2 rounded-lg border ${
        active
          ? "bg-evap-cyan/5 border-evap-cyan/40"
          : "bg-evap-surface border-evap-border"
      }`}
    >
      <div className="flex items-center justify-between">
        <span className="text-xs font-semibold text-zinc-200">{name}</span>
        {active ? (
          <span className="text-[9px] px-1.5 py-0.5 rounded-full bg-evap-cyan/10 text-evap-cyan border border-evap-cyan/30">
            Active
          </span>
        ) : (
          <span className="text-[9px] px-1.5 py-0.5 rounded-full text-zinc-500 border border-evap-border">
            {id}
          </span>
        )}
      </div>
      <p className="text-xs text-zinc-500 mt-1 leading-snug">{blurb}</p>
    </div>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div>
      <p className="text-[9px] text-zinc-500 uppercase tracking-wider mb-0.5">{label}</p>
      {children}
    </div>
  );
}
