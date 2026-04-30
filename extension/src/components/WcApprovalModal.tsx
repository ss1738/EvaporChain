/**
 * WalletConnect session approval overlay.
 * Shown when a dApp sends a connection proposal.
 */

import type { WcSessionProposal } from "@/utils/walletconnect";

interface Props {
  proposal: WcSessionProposal;
  onApprove: () => void;
  onReject: () => void;
}

export function WcApprovalModal({ proposal, onApprove, onReject }: Props) {
  const { metadata } = proposal.params.proposer;
  const requiredNs = proposal.params.requiredNamespaces ?? {};
  const optionalNs = proposal.params.optionalNamespaces ?? {};
  // Prefer the EvaporChain namespace; fall back to any required, then optional.
  const evapNs =
    requiredNs["evap"] ??
    Object.values(requiredNs)[0] ??
    optionalNs["evap"] ??
    Object.values(optionalNs)[0];

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm">
      <div className="mx-4 w-full max-w-sm rounded-xl bg-evap-surface border border-evap-border p-5 shadow-2xl">
        {/* Header */}
        <h2 className="text-sm font-semibold text-zinc-100 text-center mb-4">
          Connection Request
        </h2>

        {/* dApp info */}
        <div className="flex items-center gap-3 mb-4">
          <div className="w-10 h-10 rounded-lg bg-evap-border flex items-center justify-center">
            {metadata.icons.length > 0 ? (
              <img
                src={metadata.icons[0]}
                alt={metadata.name}
                className="w-8 h-8 rounded"
              />
            ) : (
              <span className="text-lg text-zinc-500">&#9741;</span>
            )}
          </div>
          <div className="flex-1 min-w-0">
            <p className="text-sm font-medium text-zinc-100 truncate">
              {metadata.name}
            </p>
            <p className="text-[10px] text-zinc-500 truncate">
              {metadata.url}
            </p>
          </div>
        </div>

        {metadata.description && (
          <p className="text-xs text-zinc-400 mb-4">{metadata.description}</p>
        )}

        {/* Requested permissions */}
        <div className="mb-4">
          <p className="text-[10px] uppercase tracking-wider text-zinc-500 mb-2">
            Requested Permissions
          </p>
          <div className="space-y-1">
            {evapNs?.methods.map((method) => (
              <PermissionRow key={method} method={method} />
            ))}
            {!evapNs && (
              <p className="text-xs text-zinc-500 italic">
                No EvaporChain permissions requested
              </p>
            )}
          </div>
        </div>

        {/* Requested chains */}
        <div className="mb-6">
          <p className="text-[10px] uppercase tracking-wider text-zinc-500 mb-2">
            Chains
          </p>
          <div className="flex flex-wrap gap-1">
            {(evapNs?.chains ?? []).map((chain) => (
              <span
                key={chain}
                className="px-2 py-0.5 rounded-full bg-evap-cyan/10 border border-evap-cyan/30 text-[10px] text-evap-cyan"
              >
                {chain}
              </span>
            ))}
          </div>
        </div>

        {/* Actions */}
        <div className="flex gap-3">
          <button
            onClick={onReject}
            className="flex-1 py-2.5 rounded-lg bg-zinc-700 hover:bg-zinc-600 text-sm font-medium text-zinc-300 transition"
          >
            Reject
          </button>
          <button
            onClick={onApprove}
            className="flex-1 py-2.5 rounded-lg bg-evap-cyan hover:bg-evap-cyan/90 text-sm font-medium text-black transition"
          >
            Approve
          </button>
        </div>
      </div>
    </div>
  );
}

// ── Helpers ──

function PermissionRow({ method }: { method: string }) {
  const labels: Record<string, { label: string; icon: string }> = {
    evap_getAccounts: { label: "View accounts", icon: "👤" },
    evap_signMessage: { label: "Sign messages", icon: "✍" },
    evap_signTransaction: { label: "Sign transactions", icon: "✍" },
    evap_sendTransaction: { label: "Send transactions", icon: "↗" },
    // legacy / pre-rename method names
    evaporchain_getAccounts: { label: "View accounts", icon: "👤" },
    evaporchain_signMessage: { label: "Sign messages", icon: "✍" },
    evaporchain_sendTransaction: { label: "Send transactions", icon: "↗" },
  };

  const info = labels[method] ?? { label: method, icon: "·" };

  return (
    <div className="flex items-center gap-2 px-3 py-1.5 rounded-lg bg-zinc-800/50">
      <span className="text-xs">{info.icon}</span>
      <span className="text-xs text-zinc-300">{info.label}</span>
    </div>
  );
}
