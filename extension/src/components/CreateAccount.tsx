import { useState } from "react";
import { useWallet } from "@/hooks/useWallet";

export function CreateAccount() {
  const { createAccount, error, loading, setView } = useWallet();
  const [name, setName] = useState("");
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [localError, setLocalError] = useState("");

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setLocalError("");

    if (!name.trim()) return setLocalError("Name is required");
    if (password.length < 8) return setLocalError("Password must be at least 8 characters");
    if (password !== confirm) return setLocalError("Passwords don't match");

    try {
      await createAccount(name.trim(), password);
    } catch {
      // Error handled by store
    }
  };

  return (
    <div className="flex flex-col items-center justify-center h-full px-8">
      <div className="w-16 h-16 rounded-full bg-gradient-to-br from-evap-cyan to-evap-purple flex items-center justify-center mb-6">
        <span className="text-2xl font-bold text-black">E</span>
      </div>
      <h1 className="text-lg font-semibold text-zinc-100 mb-1">Create Wallet</h1>
      <p className="text-xs text-zinc-500 mb-6">Your first post-quantum wallet</p>

      <form onSubmit={handleSubmit} className="w-full space-y-3">
        <input
          type="text"
          placeholder="Account name (e.g. main)"
          value={name}
          onChange={e => setName(e.target.value)}
          className="w-full px-4 py-3 rounded-lg bg-evap-surface border border-evap-border text-sm text-zinc-200 placeholder-zinc-600 focus:outline-none focus:border-evap-cyan transition"
          autoFocus
        />
        <input
          type="password"
          placeholder="Password (min 8 characters)"
          value={password}
          onChange={e => setPassword(e.target.value)}
          className="w-full px-4 py-3 rounded-lg bg-evap-surface border border-evap-border text-sm text-zinc-200 placeholder-zinc-600 focus:outline-none focus:border-evap-cyan transition"
        />
        <input
          type="password"
          placeholder="Confirm password"
          value={confirm}
          onChange={e => setConfirm(e.target.value)}
          className="w-full px-4 py-3 rounded-lg bg-evap-surface border border-evap-border text-sm text-zinc-200 placeholder-zinc-600 focus:outline-none focus:border-evap-cyan transition"
        />

        {(localError || error) && (
          <p className="text-xs text-evap-red">{localError || error}</p>
        )}

        <button
          type="submit"
          disabled={loading}
          className="w-full py-3 rounded-lg bg-gradient-to-r from-evap-cyan to-evap-purple text-sm font-semibold text-black hover:opacity-90 transition disabled:opacity-50"
        >
          {loading ? "Creating..." : "Create Wallet"}
        </button>
      </form>

      <button
        onClick={() => setView("import")}
        className="mt-4 text-xs text-evap-cyan hover:underline"
      >
        Already have a wallet? Import instead
      </button>

      <div className="mt-4 px-4 py-2 rounded-md bg-evap-surface border border-evap-border">
        <p className="text-xs text-zinc-500 text-center">
          Keys are encrypted with AES-256-GCM and never leave your browser.
        </p>
      </div>
    </div>
  );
}
