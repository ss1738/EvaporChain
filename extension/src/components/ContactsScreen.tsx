import { useEffect, useState } from "react";
import { RotateCw } from "lucide-react";
import { useWallet } from "@/hooks/useWallet";
import type { Contact } from "@/utils/contacts";

/**
 * Contacts / address-book screen.
 *
 * Lists saved recipient addresses with their labels and optional
 * notes. Users can add a new contact, edit an existing one inline, or
 * delete one. Visual language mirrors RefreshPoolScreen — light
 * sectioned cards, emerald/zinc/cyan accents, tabular rows.
 *
 * Persistence lives in `extension/src/utils/contacts.ts` (chrome.storage.local
 * with localStorage fallback). The store action `addContact` /
 * `removeContact` rewrites the persisted list and refreshes the
 * in-memory `contacts` slice.
 */
export function ContactsScreen() {
  const {
    contacts,
    refreshContacts,
    addContact,
    removeContact,
    setView,
  } = useWallet();

  // Form state — `editingAddr` is null for "add new", or the address of
  // the contact currently being edited (used as the form key so React
  // resets state when the user switches edit targets).
  const [editingAddr, setEditingAddr] = useState<string | null>(null);
  const [draftAddr, setDraftAddr] = useState("");
  const [draftLabel, setDraftLabel] = useState("");
  const [draftNote, setDraftNote] = useState("");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    refreshContacts();
  }, [refreshContacts]);

  const startEdit = (c: Contact) => {
    setEditingAddr(c.address);
    setDraftAddr(c.address);
    setDraftLabel(c.label);
    setDraftNote(c.note ?? "");
    setError(null);
  };

  const resetForm = () => {
    setEditingAddr(null);
    setDraftAddr("");
    setDraftLabel("");
    setDraftNote("");
    setError(null);
  };

  const handleSave = async (e: React.FormEvent) => {
    e.preventDefault();
    const addr = draftAddr.trim();
    const label = draftLabel.trim();
    if (!addr) {
      setError("Address is required");
      return;
    }
    if (!label) {
      setError("Label is required");
      return;
    }
    try {
      await addContact({
        address: addr,
        label,
        note: draftNote.trim() || undefined,
        addedAt: Date.now(),
      });
      resetForm();
    } catch (err: any) {
      setError(err?.message ?? "Failed to save contact");
    }
  };

  const handleDelete = async (addr: string) => {
    await removeContact(addr);
    if (editingAddr === addr) resetForm();
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
          <h1 className="text-sm font-semibold text-zinc-200">Contacts</h1>
          <p className="text-xs text-zinc-500">
            Saved recipient addresses with labels and notes.
          </p>
        </div>
        <button
          onClick={() => refreshContacts()}
          className="text-xs px-2 py-1 rounded text-evap-cyan border border-evap-cyan/30 hover:border-evap-cyan/60 transition"
        ><RotateCw className="w-3.5 h-3.5" strokeWidth={1.5} /></button>
      </div>

      <div className="flex-1 overflow-y-auto">
        {/* Add / edit form */}
        <form
          key={editingAddr ?? "new"}
          onSubmit={handleSave}
          className="mx-4 mt-3 px-3 py-3 rounded-lg bg-evap-surface border border-evap-border space-y-2"
        >
          <p className="text-xs text-zinc-400 font-semibold uppercase tracking-wider">
            {editingAddr ? "Edit contact" : "Add contact"}
          </p>
          <input
            type="text"
            placeholder="Address (0x…)"
            value={draftAddr}
            onChange={(e) => setDraftAddr(e.target.value)}
            disabled={editingAddr != null}
            className="w-full px-3 py-2 rounded-md bg-evap-bg border border-evap-border text-xs text-zinc-200 placeholder-zinc-600 focus:outline-none focus:border-evap-cyan transition font-mono disabled:opacity-60"
          />
          <input
            type="text"
            placeholder="Label, e.g. Alice's hot wallet"
            value={draftLabel}
            onChange={(e) => setDraftLabel(e.target.value)}
            className="w-full px-3 py-2 rounded-md bg-evap-bg border border-evap-border text-xs text-zinc-200 placeholder-zinc-600 focus:outline-none focus:border-evap-cyan transition"
          />
          <input
            type="text"
            placeholder="Note (optional)"
            value={draftNote}
            onChange={(e) => setDraftNote(e.target.value)}
            className="w-full px-3 py-2 rounded-md bg-evap-bg border border-evap-border text-xs text-zinc-200 placeholder-zinc-600 focus:outline-none focus:border-evap-cyan transition"
          />
          {error && <p className="text-xs text-evap-red">{error}</p>}
          <div className="flex items-center gap-2">
            <button
              type="submit"
              className="flex-1 py-2 rounded-md bg-gradient-to-r from-evap-cyan to-evap-purple text-xs font-semibold text-black hover:opacity-90 transition"
            >
              {editingAddr ? "Save changes" : "Add contact"}
            </button>
            {editingAddr && (
              <button
                type="button"
                onClick={resetForm}
                className="px-3 py-2 rounded-md bg-evap-surface border border-evap-border text-xs text-zinc-300 hover:border-evap-cyan/40 transition"
              >
                Cancel
              </button>
            )}
          </div>
        </form>

        {/* Contact list */}
        <div className="mx-4 mt-4 mb-4">
          <p className="text-xs text-zinc-400 font-semibold mb-2 uppercase tracking-wider">
            Saved contacts ({contacts.length})
          </p>
          {contacts.length === 0 ? (
            <p className="text-xs text-zinc-600 text-center py-8">
              No contacts saved yet.
            </p>
          ) : (
            <div className="rounded-lg border border-evap-border overflow-hidden divide-y divide-evap-border">
              {contacts.map((c) => (
                <div
                  key={c.address}
                  className="px-3 py-2 bg-evap-bg hover:bg-evap-surface/60 transition"
                >
                  <div className="flex items-center justify-between gap-2">
                    <div className="min-w-0 flex-1">
                      <p className="text-xs font-semibold text-zinc-200 truncate">
                        {c.label}
                      </p>
                      <p
                        className="text-xs font-mono text-zinc-500 truncate"
                        title={c.address}
                      >
                        {c.address}
                      </p>
                      {c.note && (
                        <p className="text-xs text-zinc-500 mt-0.5 truncate">
                          {c.note}
                        </p>
                      )}
                    </div>
                    <div className="flex items-center gap-1 shrink-0">
                      <button
                        onClick={() => startEdit(c)}
                        className="text-xs px-2 py-1 rounded border border-evap-border text-zinc-400 hover:text-zinc-200 hover:border-evap-cyan/40 transition"
                      >
                        Edit
                      </button>
                      <button
                        onClick={() => handleDelete(c.address)}
                        className="text-xs px-2 py-1 rounded border border-evap-red/30 text-evap-red hover:bg-evap-red/10 transition"
                      >
                        Delete
                      </button>
                    </div>
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>

        <div className="mx-4 mb-4 px-3 py-2 rounded-lg bg-evap-cyan/5 border border-evap-cyan/20">
          <p className="text-[9px] text-zinc-400 leading-snug">
            Contacts are stored locally in your browser via
            chrome.storage.local. They never leave your device. The
            send screen autocompletes from this list as you type.
          </p>
        </div>
      </div>
    </div>
  );
}
