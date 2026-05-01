import { useMemo, useState } from "react";
import { useWallet } from "@/hooks/useWallet";
import { formatBalance } from "@/utils/format";
import { Header } from "./Header";
import { TxSimulation } from "./TxSimulation";
import { LadVmPreview } from "./LadVmPreview";
import { api } from "@/utils/api";
import { findContactSync } from "@/utils/contacts";

type SendStep = "form" | "preview" | "sent";
type SendMode = "address" | "object";

export function SendScreen() {
  const {
    sendTransfer,
    balance,
    setView,
    loading,
    error,
    chainStatus,
    objects,
    activeAccount,
    shardsHealth,
    addressShard,
    contacts,
    addContact,
  } = useWallet();
  const [mode, setMode] = useState<SendMode>("address");
  const [to, setTo] = useState("");
  const [objectId, setObjectId] = useState("");
  const [amount, setAmount] = useState("");
  const [step, setStep] = useState<SendStep>("form");
  // Address-book autocomplete + "save as contact" prompt state.
  const [showSuggestions, setShowSuggestions] = useState(false);
  const [saveContactOpen, setSaveContactOpen] = useState(false);
  const [saveContactLabel, setSaveContactLabel] = useState("");
  const [saveContactDismissed, setSaveContactDismissed] = useState(false);

  // In "object" mode, the user picks one of their own objects; in
  // "address" mode they paste a recipient address. Owned objects are
  // filtered against the active account so the dropdown only shows
  // things the user can actually send.
  const ownedObjects = activeAccount
    ? objects.filter((o) => o.owner === activeAccount.address)
    : [];
  const targetObject = mode === "object"
    ? objects.find((o) => o.id === objectId) ?? null
    : null;

  const parsedAmount = parseInt(amount, 10);
  const recipientReady = mode === "address" ? to.length > 0 : objectId.length > 0;
  const isFormValid = recipientReady && !isNaN(parsedAmount) && parsedAmount > 0;

  // Contact autocomplete: filter the address book by partial match on
  // either label or address (case-insensitive). Suggestions only show
  // when the user is typing into the address input and they don't
  // exactly match an existing contact.
  const knownContact = mode === "address" ? findContactSync(contacts, to) : null;
  const suggestions = useMemo(() => {
    if (mode !== "address") return [];
    const q = to.trim().toLowerCase();
    if (!q) return [];
    if (knownContact) return [];
    return contacts
      .filter(
        (c) =>
          c.address.toLowerCase().includes(q) ||
          c.label.toLowerCase().includes(q),
      )
      .slice(0, 5);
  }, [contacts, to, mode, knownContact]);

  // Cross-shard awareness. Resolve the recipient address (`to` in
  // address mode, the picked object's owner in object mode) and
  // compare its shard against the sender's shard. Single-shard chains
  // and "sharding disabled" responses both leave this null so the
  // banner stays hidden.
  const recipientAddrForShard =
    mode === "address" ? to : targetObject?.owner ?? "";
  const crossShardInfo = useMemo<{
    sender: number;
    recipient: number;
  } | null>(() => {
    if (!shardsHealth?.active) return null;
    if (shardsHealth.total_shards <= 1) return null;
    if (addressShard == null) return null;
    if (!recipientAddrForShard) return null;
    const r = api.computeShardForAddress(
      recipientAddrForShard,
      shardsHealth.total_shards,
    );
    if (r == null) return null;
    if (r.shard_id === addressShard) return null;
    return { sender: addressShard, recipient: r.shard_id };
  }, [shardsHealth, addressShard, recipientAddrForShard]);

  const handlePreview = (e: React.FormEvent) => {
    e.preventDefault();
    if (!isFormValid) return;
    setStep("preview");
  };

  // Resolve recipient address: in "address" mode it's the typed value;
  // in "object" mode it's the picked object's owner.
  const resolvedTo = mode === "object" ? targetObject?.owner ?? "" : to;

  const handleConfirm = async () => {
    if (!resolvedTo || !amount) return;
    const result = await sendTransfer(resolvedTo, parsedAmount);
    if (result.success) {
      // If the recipient address isn't in the contacts book and the
      // user hasn't dismissed the prompt, offer to save it. We delay
      // moving back to home so the inline prompt has time to render.
      const recipientKnown = !!findContactSync(contacts, resolvedTo);
      if (!recipientKnown && !saveContactDismissed && mode === "address") {
        setSaveContactOpen(true);
        setSaveContactLabel("");
        setStep("sent");
        // Don't auto-redirect; let the user save or dismiss.
        return;
      }
      setStep("sent");
      setTimeout(() => setView("home"), 2000);
    }
  };

  const handleSaveContact = async () => {
    const label = saveContactLabel.trim();
    if (!label) return;
    await addContact({
      address: resolvedTo,
      label,
      addedAt: Date.now(),
    });
    setSaveContactOpen(false);
    setTimeout(() => setView("home"), 1000);
  };

  const handleDismissSaveContact = () => {
    setSaveContactOpen(false);
    setSaveContactDismissed(true);
    setTimeout(() => setView("home"), 800);
  };

  const handleCancelPreview = () => {
    setStep("form");
  };

  // Success screen
  if (step === "sent") {
    return (
      <div className="flex flex-col h-full">
        <Header />
        <div className="flex flex-col items-center justify-center flex-1 px-8">
          <div className="w-16 h-16 rounded-full bg-evap-green/20 flex items-center justify-center mb-4">
            <span className="text-3xl">&#10003;</span>
          </div>
          <p className="text-sm font-semibold text-zinc-200">Transaction Sent</p>
          <p className="text-xs text-zinc-500 mt-1">{amount} EVAP sent</p>

          {saveContactOpen && (
            <div className="mt-6 w-full max-w-sm px-4 py-3 rounded-lg bg-evap-surface border border-evap-cyan/30 space-y-2">
              <p className="text-[11px] text-zinc-300 font-semibold">
                Save as contact?
              </p>
              <p className="text-[10px] text-zinc-500 font-mono truncate">
                {resolvedTo}
              </p>
              <input
                type="text"
                placeholder="Label, e.g. Alice's hot wallet"
                value={saveContactLabel}
                onChange={(e) => setSaveContactLabel(e.target.value)}
                autoFocus
                className="w-full px-3 py-2 rounded-md bg-evap-bg border border-evap-border text-[11px] text-zinc-200 placeholder-zinc-600 focus:outline-none focus:border-evap-cyan transition"
              />
              <div className="flex items-center gap-2">
                <button
                  onClick={handleSaveContact}
                  disabled={!saveContactLabel.trim()}
                  className="flex-1 py-2 rounded-md bg-gradient-to-r from-evap-cyan to-evap-purple text-[11px] font-semibold text-black hover:opacity-90 transition disabled:opacity-50"
                >
                  Save contact
                </button>
                <button
                  onClick={handleDismissSaveContact}
                  className="px-3 py-2 rounded-md bg-evap-surface border border-evap-border text-[11px] text-zinc-300 hover:border-evap-cyan/40 transition"
                >
                  Skip
                </button>
              </div>
            </div>
          )}
        </div>
      </div>
    );
  }

  // Preview / simulation step
  if (step === "preview") {
    const activeAddress = useWallet.getState().activeAccount?.address ?? "";
    const currentEpoch = chainStatus?.epoch ?? 0;
    // Default epoch duration: 30 seconds (configurable per chain)
    const epochDurationMs = 30_000;

    return (
      <div className="flex flex-col h-full">
        <Header />
        <div className="px-4 pt-4">
          <button
            onClick={handleCancelPreview}
            className="text-xs text-zinc-500 hover:text-zinc-300 mb-3"
          >
            &larr; Back to form
          </button>
          <h2 className="text-lg font-semibold text-zinc-100 mb-1">Preview Transaction</h2>
          <p className="text-xs text-zinc-500 mb-4">
            Review the simulation before sending
          </p>
        </div>

        <div className="px-4 flex-1 overflow-y-auto pb-4">
          {error && (
            <div className="mb-3 rounded-md bg-evap-red/10 border border-evap-red/30 px-3 py-2">
              <p className="text-[10px] text-evap-red">{error}</p>
            </div>
          )}

          <TxSimulation
            from={activeAddress}
            to={resolvedTo}
            amount={parsedAmount}
            currentBalance={balance}
            currentEpoch={currentEpoch}
            epochDurationMs={epochDurationMs}
            onConfirm={handleConfirm}
            onCancel={handleCancelPreview}
          />

          {/* LAD-VM substructural-resource preview. Now wired to the
              target object's `is_lad_typed` marker; only renders in
              "object" mode for objects that actually carry a LAD mode.
              See LadVmPreview.tsx file header. */}
          {targetObject?.is_lad_typed === true && (
            <LadVmPreview initialMode={targetObject.lad_mode} />
          )}

          {loading && (
            <div className="mt-3 flex items-center justify-center gap-2">
              <div className="w-4 h-4 border-2 border-evap-cyan border-t-transparent rounded-full animate-spin" />
              <span className="text-[11px] text-zinc-400">Sending transaction...</span>
            </div>
          )}
        </div>
      </div>
    );
  }

  // Form step
  return (
    <div className="flex flex-col h-full">
      <Header />
      <div className="px-4 pt-4">
        <button
          onClick={() => setView("home")}
          className="text-xs text-zinc-500 hover:text-zinc-300 mb-3"
        >
          &larr; Back
        </button>
        <h2 className="text-lg font-semibold text-zinc-100 mb-1">Send EVAP</h2>
        <p className="text-xs text-zinc-500 mb-4">
          Available: {formatBalance(balance)} EVAP
        </p>
      </div>

      <form onSubmit={handlePreview} className="px-4 space-y-3 flex-1">
        {/* Mode toggle: address vs object. Object mode pulls a target
            from the user's owned-objects list and, when LAD-typed,
            unlocks the LadVmPreview surface in the preview step. */}
        <div className="flex items-center gap-3 text-[10px] text-zinc-400">
          <label className="flex items-center gap-1.5 cursor-pointer">
            <input
              type="radio"
              name="send-mode"
              value="address"
              checked={mode === "address"}
              onChange={() => setMode("address")}
              className="accent-evap-cyan"
            />
            <span>Address</span>
          </label>
          <label className="flex items-center gap-1.5 cursor-pointer">
            <input
              type="radio"
              name="send-mode"
              value="object"
              checked={mode === "object"}
              onChange={() => setMode("object")}
              className="accent-evap-cyan"
            />
            <span>Object</span>
          </label>
        </div>

        {mode === "address" ? (
          <div className="relative">
            <label className="text-[10px] text-zinc-500 mb-1 block">
              Recipient Address
              {knownContact && (
                <span className="ml-2 text-evap-cyan font-semibold">
                  · {knownContact.label}
                </span>
              )}
            </label>
            <input
              type="text"
              placeholder="0x... or contact label"
              value={to}
              onChange={e => {
                setTo(e.target.value);
                setShowSuggestions(true);
              }}
              onFocus={() => setShowSuggestions(true)}
              onBlur={() => {
                // Delay so click events on suggestions can register
                // before the dropdown unmounts.
                setTimeout(() => setShowSuggestions(false), 150);
              }}
              className="w-full px-4 py-3 rounded-lg bg-evap-surface border border-evap-border text-sm text-zinc-200 placeholder-zinc-600 focus:outline-none focus:border-evap-cyan transition font-mono"
              autoFocus
            />
            {showSuggestions && suggestions.length > 0 && (
              <div className="absolute z-10 left-0 right-0 mt-1 rounded-lg bg-evap-surface border border-evap-border overflow-hidden divide-y divide-evap-border">
                {suggestions.map((c) => (
                  <button
                    key={c.address}
                    type="button"
                    onMouseDown={(e) => {
                      // mousedown fires before blur, so onClick would
                      // be cancelled by the input losing focus.
                      e.preventDefault();
                      setTo(c.address);
                      setShowSuggestions(false);
                    }}
                    className="w-full text-left px-3 py-2 hover:bg-evap-bg transition"
                  >
                    <p className="text-[11px] font-semibold text-zinc-200 truncate">
                      {c.label}
                    </p>
                    <p className="text-[10px] font-mono text-zinc-500 truncate">
                      {c.address}
                    </p>
                  </button>
                ))}
              </div>
            )}
          </div>
        ) : (
          <div>
            <label className="text-[10px] text-zinc-500 mb-1 block">Target Object</label>
            <select
              value={objectId}
              onChange={e => setObjectId(e.target.value)}
              className="w-full px-4 py-3 rounded-lg bg-evap-surface border border-evap-border text-sm text-zinc-200 focus:outline-none focus:border-evap-cyan transition"
              autoFocus
            >
              <option value="">{ownedObjects.length === 0 ? "No owned objects" : "Select an object…"}</option>
              {ownedObjects.map((o) => (
                <option key={o.id} value={o.id}>
                  {o.name} {o.is_lad_typed ? `· LAD/${o.lad_mode}` : ""}
                </option>
              ))}
            </select>
            {targetObject && (
              <p className="text-[10px] text-zinc-500 mt-1 font-mono truncate">
                Owner: {targetObject.owner}
              </p>
            )}
          </div>
        )}

        {crossShardInfo && (
          <div className="px-3 py-2 rounded-lg bg-evap-cyan/5 border border-evap-cyan/30">
            <p className="text-[11px] text-evap-cyan">
              Cross-shard transfer: shard {crossShardInfo.sender} → shard {crossShardInfo.recipient}. May take longer to finalise.
            </p>
          </div>
        )}

        <div>
          <label className="text-[10px] text-zinc-500 mb-1 block">Amount</label>
          <div className="relative">
            <input
              type="number"
              placeholder="0"
              min="1"
              max={balance}
              value={amount}
              onChange={e => setAmount(e.target.value)}
              className="w-full px-4 py-3 rounded-lg bg-evap-surface border border-evap-border text-sm text-zinc-200 placeholder-zinc-600 focus:outline-none focus:border-evap-cyan transition"
            />
            <button
              type="button"
              onClick={() => setAmount(String(balance))}
              className="absolute right-2 top-1/2 -translate-y-1/2 text-[10px] text-evap-cyan hover:underline"
            >
              MAX
            </button>
          </div>
        </div>

        {error && <p className="text-xs text-evap-red">{error}</p>}

        <button
          type="submit"
          disabled={!isFormValid}
          className="w-full py-3 rounded-lg bg-gradient-to-r from-evap-cyan to-evap-purple text-sm font-semibold text-black hover:opacity-90 transition disabled:opacity-50"
        >
          Preview Transaction
        </button>
      </form>
    </div>
  );
}
