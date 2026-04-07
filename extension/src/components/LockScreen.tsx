import { useState } from "react";
import { useWallet } from "@/hooks/useWallet";

export function LockScreen() {
  const { unlock, error, loading } = useWallet();
  const [password, setPassword] = useState("");

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!password) return;
    await unlock(password);
  };

  return (
    <div className="flex flex-col items-center justify-center h-full px-8">
      <div className="w-16 h-16 rounded-full bg-gradient-to-br from-evap-cyan to-evap-purple flex items-center justify-center mb-6">
        <span className="text-2xl font-bold text-black">E</span>
      </div>
      <h1 className="text-lg font-semibold text-zinc-100 mb-1">EvaporChain Wallet</h1>
      <p className="text-xs text-zinc-500 mb-6">Enter password to unlock</p>

      <form onSubmit={handleSubmit} className="w-full space-y-3">
        <input
          type="password"
          placeholder="Password"
          value={password}
          onChange={e => setPassword(e.target.value)}
          className="w-full px-4 py-3 rounded-lg bg-evap-surface border border-evap-border text-sm text-zinc-200 placeholder-zinc-600 focus:outline-none focus:border-evap-cyan transition"
          autoFocus
        />
        {error && <p className="text-xs text-evap-red">{error}</p>}
        <button
          type="submit"
          disabled={loading || !password}
          className="w-full py-3 rounded-lg bg-gradient-to-r from-evap-cyan to-evap-purple text-sm font-semibold text-black hover:opacity-90 transition disabled:opacity-50"
        >
          {loading ? "Unlocking..." : "Unlock"}
        </button>
      </form>

      <div className="mt-8 px-4 py-2 rounded-md bg-evap-surface border border-evap-border">
        <p className="text-[10px] text-zinc-500 text-center">
          🛡️ Post-Quantum Secured · ML-DSA (FIPS 204)
        </p>
      </div>
    </div>
  );
}
