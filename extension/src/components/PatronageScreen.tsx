import { useEffect, useMemo, useState } from "react";
import { useWallet } from "@/hooks/useWallet";
import type { StateObject } from "@/utils/api";

/**
 * Patronage Covenants — pre-fund eviction immunity for your objects.
 *
 * Surfaces the chain's five /api/patronage/* endpoints. Visual language
 * matches EnergyDashboard: header bar, summary card, per-object rows
 * with inline pledge form. The namespace_id_hex is auto-filled from
 * the global /api/patronage/status response (patronage_ns_hex), which
 * is the demo namespace seeded by the node — replaceable per-pledge.
 */
export function PatronageScreen() {
  const {
    objects,
    chainStatus,
    setView,
    refreshObjects,
    refreshChainStatus,
    refreshPatronage,
    refreshPatronageImmunity,
    patronageStatus,
    patronageImmunities,
    pledgePatronage,
    honourPatronage,
    revokePatronage,
    error,
    notification,
    loading,
    setNotification,
    setError,
  } = useWallet();

  const [openPledgeId, setOpenPledgeId] = useState<string | null>(null);

  // Use chain epoch for the immunity probe + as the default current_epoch.
  const currentEpoch = chainStatus?.epoch ?? 0;

  // Object IDs in the wallet are returned as `0x…` hex; the patronage
  // API expects raw hex without the `0x` prefix. Normalise once here.
  const stripHex = (s: string): string => (s.startsWith("0x") ? s.slice(2) : s);

  useEffect(() => {
    refreshChainStatus();
    refreshObjects();
    refreshPatronage();
  }, [refreshChainStatus, refreshObjects, refreshPatronage]);

  // Probe immunity for every visible object on mount + whenever the
  // object list or current epoch changes. We do this serially via the
  // store (each call lands in patronageImmunities[objectIdHex]).
  useEffect(() => {
    if (!objects.length || currentEpoch < 0) return;
    let cancelled = false;
    (async () => {
      for (const obj of objects) {
        if (cancelled) return;
        await refreshPatronageImmunity(stripHex(obj.id), currentEpoch);
      }
    })();
    return () => { cancelled = true; };
  }, [objects, currentEpoch, refreshPatronageImmunity]);

  // Derive default namespace from the global status response.
  const defaultNamespaceHex = patronageStatus?.patronage_ns_hex ?? "";

  return (
    <div className="flex flex-col h-full bg-evap-bg">
      {/* Header */}
      <div className="flex items-center gap-3 px-4 py-3 border-b border-evap-border">
        <button
          onClick={() => setView("objects")}
          className="text-zinc-400 hover:text-zinc-200 transition text-sm"
        >
          ←
        </button>
        <div className="flex-1">
          <h1 className="text-sm font-semibold text-zinc-200">Patronage Covenants</h1>
          <p className="text-[10px] text-zinc-500">Pre-fund eviction immunity for your objects.</p>
        </div>
      </div>

      <div className="flex-1 overflow-y-auto">
        {/* Notification / error banners */}
        {notification && (
          <div className="mx-4 mt-3 px-3 py-2 rounded-lg bg-evap-green/10 border border-evap-green/30 flex items-start justify-between gap-2">
            <p className="text-[11px] text-evap-green">{notification}</p>
            <button onClick={() => setNotification(null)} className="text-[10px] text-evap-green/60 hover:text-evap-green">×</button>
          </div>
        )}
        {error && (
          <div className="mx-4 mt-3 px-3 py-2 rounded-lg bg-evap-red/10 border border-evap-red/30 flex items-start justify-between gap-2">
            <p className="text-[11px] text-evap-red">{error}</p>
            <button onClick={() => setError(null)} className="text-[10px] text-evap-red/60 hover:text-evap-red">×</button>
          </div>
        )}

        {/* Pool status card */}
        <div className="mx-4 mt-3 px-3 py-3 rounded-lg bg-evap-surface border border-evap-border">
          <p className="text-[10px] text-zinc-400 font-semibold mb-2 uppercase tracking-wider">
            Patronage Pool — Epoch {currentEpoch}
          </p>
          {patronageStatus ? (
            <div className="grid grid-cols-3 gap-2">
              <PoolStat label="Active covenants" value={patronageStatus.active_covenants.toString()} />
              <PoolStat label="Pre-funded" value={patronageStatus.total_pre_funded.toLocaleString()} sub="EVAP" />
              <PoolStat label="Total score" value={patronageStatus.total_active_score.toLocaleString()} />
            </div>
          ) : (
            <p className="text-[11px] text-zinc-500">Loading pool status…</p>
          )}
          {patronageStatus?.patronage_ns_hex && (
            <p className="text-[9px] text-zinc-600 font-mono mt-2 truncate" title={patronageStatus.patronage_ns_hex}>
              ns: {patronageStatus.patronage_ns_hex.slice(0, 32)}…
            </p>
          )}
        </div>

        {/* Object list */}
        <div className="mx-4 mt-4 mb-4">
          <p className="text-[10px] text-zinc-400 font-semibold mb-2 uppercase tracking-wider">
            Your objects
          </p>
          {objects.length === 0 ? (
            <p className="text-[11px] text-zinc-600 text-center py-8">
              No objects to patronise.
            </p>
          ) : (
            <div className="space-y-1.5">
              {objects.map((obj) => {
                const idHex = stripHex(obj.id);
                const immunity = patronageImmunities[idHex];
                return (
                  <PatronageObjectRow
                    key={obj.id}
                    obj={obj}
                    idHex={idHex}
                    immunity={immunity}
                    currentEpoch={currentEpoch}
                    defaultNamespaceHex={defaultNamespaceHex}
                    isPledgeOpen={openPledgeId === obj.id}
                    onTogglePledge={() => setOpenPledgeId(openPledgeId === obj.id ? null : obj.id)}
                    onPledge={async (donationPerEpoch, epochs, namespaceHex) => {
                      const resp = await pledgePatronage({
                        object_id_hex: idHex,
                        namespace_id_hex: namespaceHex,
                        donation_per_epoch: donationPerEpoch,
                        epochs,
                        current_epoch: currentEpoch,
                      });
                      if (resp.status === "pledged") setOpenPledgeId(null);
                    }}
                    onHonour={async () => {
                      await honourPatronage({ object_id_hex: idHex, epoch: currentEpoch });
                    }}
                    onRevoke={async () => {
                      await revokePatronage({ object_id_hex: idHex, epoch: currentEpoch });
                    }}
                    busy={loading}
                  />
                );
              })}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

// ─────────── Sub-components ───────────────────────────────────────

function PoolStat({ label, value, sub }: { label: string; value: string; sub?: string }) {
  return (
    <div>
      <p className="text-[9px] text-zinc-500 uppercase tracking-wider">{label}</p>
      <div className="flex items-baseline gap-1 mt-0.5">
        <span className="text-sm font-bold text-zinc-200">{value}</span>
        {sub && <span className="text-[9px] text-zinc-600">{sub}</span>}
      </div>
    </div>
  );
}

interface ObjectRowProps {
  obj: StateObject;
  idHex: string;
  immunity?: { immune: boolean; patronage_score: number; epoch: number };
  currentEpoch: number;
  defaultNamespaceHex: string;
  isPledgeOpen: boolean;
  onTogglePledge: () => void;
  onPledge: (donationPerEpoch: number, epochs: number, namespaceHex: string) => Promise<void>;
  onHonour: () => Promise<void>;
  onRevoke: () => Promise<void>;
  busy: boolean;
}

function PatronageObjectRow({
  obj,
  idHex,
  immunity,
  currentEpoch,
  defaultNamespaceHex,
  isPledgeOpen,
  onTogglePledge,
  onPledge,
  onHonour,
  onRevoke,
  busy,
}: ObjectRowProps) {
  // Pledge form state — kept local to the row so each can be edited
  // independently. Defaults are conservative (10/10/seeded ns).
  const [donationPerEpoch, setDonationPerEpoch] = useState<string>("100");
  const [epochs, setEpochs] = useState<string>("10");
  const [namespaceHex, setNamespaceHex] = useState<string>(defaultNamespaceHex);

  // Keep namespace in sync if the global default arrives later.
  useMemo(() => {
    if (!namespaceHex && defaultNamespaceHex) setNamespaceHex(defaultNamespaceHex);
  }, [defaultNamespaceHex, namespaceHex]);

  const isImmune = immunity?.immune === true;

  return (
    <div className="rounded-lg bg-evap-surface border border-evap-border">
      <div className="flex items-center gap-2 px-3 py-2.5">
        <div className="flex-1 min-w-0">
          <div className="flex items-center justify-between gap-2">
            <span className="text-[11px] font-medium text-zinc-200 truncate">
              {obj.name || obj.id.slice(0, 12)}
            </span>
            {isImmune ? (
              <span className="text-[9px] px-1.5 py-0.5 rounded-full bg-evap-green/10 text-evap-green border border-evap-green/30 shrink-0">
                Immune · score {immunity?.patronage_score ?? 0}
              </span>
            ) : (
              <span className="text-[9px] px-1.5 py-0.5 rounded-full bg-zinc-700/40 text-zinc-400 border border-zinc-600/40 shrink-0">
                No covenant
              </span>
            )}
          </div>
          <p className="text-[9px] text-zinc-600 font-mono truncate mt-0.5">{obj.id}</p>
        </div>
      </div>
      <div className="flex gap-1 px-3 pb-2">
        <button
          onClick={onTogglePledge}
          disabled={busy}
          className="flex-1 px-2 py-1 rounded text-[10px] font-medium bg-evap-cyan/10 text-evap-cyan border border-evap-cyan/30 hover:border-evap-cyan/60 transition disabled:opacity-50"
        >
          {isPledgeOpen ? "Close" : isImmune ? "Renew pledge" : "Pledge"}
        </button>
        {isImmune && (
          <>
            <button
              onClick={onHonour}
              disabled={busy}
              className="flex-1 px-2 py-1 rounded text-[10px] font-medium bg-evap-purple/10 text-evap-purple border border-evap-purple/30 hover:border-evap-purple/60 transition disabled:opacity-50"
            >
              Honour
            </button>
            <button
              onClick={onRevoke}
              disabled={busy}
              className="flex-1 px-2 py-1 rounded text-[10px] font-medium bg-evap-red/10 text-evap-red border border-evap-red/30 hover:border-evap-red/60 transition disabled:opacity-50"
            >
              Revoke
            </button>
          </>
        )}
      </div>

      {isPledgeOpen && (
        <div className="border-t border-evap-border px-3 py-3 space-y-2">
          <FormField label="Donation per epoch (EVAP)">
            <input
              type="number"
              min="1"
              value={donationPerEpoch}
              onChange={(e) => setDonationPerEpoch(e.target.value)}
              className="w-full bg-evap-bg border border-evap-border rounded px-2 py-1 text-[11px] text-zinc-200 focus:outline-none focus:border-evap-cyan/60"
            />
          </FormField>
          <FormField label="Epochs">
            <input
              type="number"
              min="1"
              value={epochs}
              onChange={(e) => setEpochs(e.target.value)}
              className="w-full bg-evap-bg border border-evap-border rounded px-2 py-1 text-[11px] text-zinc-200 focus:outline-none focus:border-evap-cyan/60"
            />
          </FormField>
          <FormField label="Current epoch (auto)">
            <input
              type="number"
              value={currentEpoch}
              readOnly
              className="w-full bg-evap-bg/50 border border-evap-border rounded px-2 py-1 text-[11px] text-zinc-500 font-mono"
            />
          </FormField>
          <FormField label="Namespace (hex)">
            <input
              type="text"
              value={namespaceHex}
              onChange={(e) => setNamespaceHex(e.target.value.replace(/^0x/, ""))}
              placeholder={defaultNamespaceHex || "namespace_id_hex"}
              className="w-full bg-evap-bg border border-evap-border rounded px-2 py-1 text-[11px] text-zinc-200 font-mono focus:outline-none focus:border-evap-cyan/60"
            />
          </FormField>
          <div className="flex justify-between items-center pt-1">
            <span className="text-[10px] text-zinc-500">
              Total pre-fund: <span className="text-zinc-300 font-semibold">
                {(Math.max(parseInt(donationPerEpoch) || 0, 0) * Math.max(parseInt(epochs) || 0, 0)).toLocaleString()}
              </span> EVAP
            </span>
            <button
              onClick={() => {
                const dpe = parseInt(donationPerEpoch);
                const ep = parseInt(epochs);
                if (!Number.isFinite(dpe) || dpe <= 0) return;
                if (!Number.isFinite(ep) || ep <= 0) return;
                if (!namespaceHex) return;
                onPledge(dpe, ep, namespaceHex);
              }}
              disabled={busy || !namespaceHex || !donationPerEpoch || !epochs}
              className="px-3 py-1 rounded text-[10px] font-semibold bg-evap-cyan text-black hover:bg-evap-cyan/90 transition disabled:opacity-50"
            >
              {busy ? "…" : "Confirm pledge"}
            </button>
          </div>
          <p className="text-[9px] text-zinc-600 leading-snug pt-1">
            Object: <span className="font-mono text-zinc-500">{idHex.slice(0, 24)}…</span>
          </p>
        </div>
      )}
    </div>
  );
}

function FormField({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div>
      <p className="text-[9px] text-zinc-500 uppercase tracking-wider mb-0.5">{label}</p>
      {children}
    </div>
  );
}
